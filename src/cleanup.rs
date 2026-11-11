//! The safe cleanup: changes to whitespace that cannot change what any
//! token is. It is how a statement the layout keeps as written is printed
//! (see [`Builder::as_written`](crate::doc::Builder::as_written)), and the
//! layout keeps its guarantees for every statement:
//!
//! - Whitespace at the end of a line is removed, and so is the `\r` of a
//!   CRLF line ending. A line comment loses the whitespace at its end, `\r`
//!   included (see [`TokenKind::LineComment`]), except that `# ` keeps its
//!   space, without which it is not a comment. In a template, characters
//!   that ClickHouse counts as whitespace and the renderer does not
//!   (U+FEFF, U+200B and a few more) stay, because they stop a `-` marker
//!   from stripping what lies beyond them.
//! - In a template whose rendering joins text (see
//!   [`render_joins_text`]), whitespace changes only where the rendered SQL
//!   puts it between the same neighbours as the template does; the rule and
//!   why it is enough are at [`inside_rendered_tokens`]. This module finds
//!   that whitespace, for statements kept as written; in statements it lays
//!   out, the layout puts every block tag on a line of its own, which keeps
//!   whitespace on both sides of it (see `crate::layout`).
//! - Blank lines collapse: at most two between statements, at most one
//!   inside a statement, and none right after `(` or before `)`.
//! - The file starts without a blank line (a byte-order mark stays) and ends
//!   with exactly one `\n`; a file of only whitespace becomes empty.
//!   ClickHouse lexes the first and last bytes of the input specially (`.`
//!   at the very start begins a number, `1e` at the very end is a bare word),
//!   so where adding whitespace there would change the first or last token,
//!   none is added.
//!
//! Nothing inside a token changes, line endings included: block comments,
//! strings, quoted identifiers, heredocs and Jinja tags spanning lines keep
//! their `\r\n`, and `INSERT` data keeps every byte. So a CRLF file with
//! such a token comes out with mixed line endings, which is the price of
//! never touching a literal's bytes. Two exceptions keep data intact at the
//! end of the file: data in a format other than `Values` that runs to the
//! end of the input gets no final newline, and data that ends at a blank
//! line keeps that blank line, since without it the data would run to the
//! end of the input and gain the final newline.
//!
//! Text the parser found ClickHouse reads as written (see
//! [`Facts::keep_verbatim`](crate::verify::Facts::keep_verbatim)) is left as
//! it is, whitespace inside it included.

use crate::lexer::Mode;
use crate::lexer::Token;
use crate::lexer::TokenKind;
use crate::lexer::is_whitespace;
use crate::lexer::jinja;
use crate::lexer::jinja::TagKind;
use crate::split::DataFormat;
use crate::split::Split;
use crate::split::split;
use crate::verify::line_comment_text;
use crate::verify::render_joins_text;
use crate::verify::rendered;

/// What a template's rendered text means for the cleanup.
#[derive(Default)]
pub(crate) struct Rendered {
    /// For each token of the template, whether the cleanup leaves it as it
    /// is; empty for a text whose rendering joins nothing.
    pub(crate) kept: Vec<bool>,
    /// For each token, whether it lies inside a token of the rendered text
    /// together with other tokens.
    pub(crate) inside: Vec<bool>,
    /// For each token, whether it lies in or after `INSERT` data that the
    /// rendered text holds or a branch can make. With
    /// [`inside`](Self::inside), the part of [`kept`](Self::kept) that does
    /// not depend on the whitespace next to block and comment tags, which the
    /// layout handles itself.
    pub(crate) data: Vec<bool>,
    /// The end of the text stays as it is: the rendered text ends in
    /// `INSERT` data in a format other than `Values`, with nothing but
    /// whitespace after it, or a branch can move where such data ends.
    pub(crate) open_data: bool,
}

/// What the cleanup leaves as it is in a template; nothing for a text
/// whose rendering joins nothing.
pub(crate) fn rendered_facts(source: &str, split: &Split, mode: Mode) -> Rendered {
    if mode == Mode::Jinja && render_joins_text(source) {
        let removable = |c: char| is_whitespace(c) && c.is_whitespace();
        inside_rendered_tokens(source, split, removable)
    } else {
        Rendered::default()
    }
}

/// Which tokens of a template the cleanup leaves as they are, because
/// changing their whitespace could change the rendered SQL.
///
/// The cleanup changes a run of whitespace (a whitespace token, or the end of
/// a line comment) in a template only where two things hold:
///
/// - **The run is not inside a token of the rendered text.** Rendering
///   removes block tags and stripped whitespace, which can turn separate
///   template tokens into one string, heredoc, comment or run of `INSERT`
///   data, such as `$a${% x %}` … `{% y %}$a$`; the whitespace inside that
///   is the literal's text.
/// - **The run keeps its left neighbour.** Where the renderer keeps the
///   run's first character, the character before it in the rendered text is
///   the one before it in the template, or whitespace with only comment tags
///   between; or nothing but whitespace comes before the run. A run that
///   directly follows a block or comment tag meets another character once
///   rendered, and that character can depend on the whitespace: `#` is a
///   comment only before a space, so `{%- x %}# {%- y %} ` renders the run
///   after `{%- y %}` as the space of `# `.
///
/// Together these make every change local: the change keeps a run's last
/// line, so the text after the run is the same as in the template, and the
/// text before it is the same character or whitespace that the change does
/// not touch; the tokens on both sides meet the neighbours they meet in the
/// template. Where a `-` marker strips the run's start, it strips the old
/// and the new whitespace alike, up to the same last line. A run of a branch
/// follows the same character in every render that includes it, because
/// the character comes from the same text between two tags, comment tags
/// render nothing in every render, and where only whitespace renders before
/// a run with every branch taken, no render puts more. The guarantee rests on the
/// rendered text the verifier checks, where every branch is taken: a
/// render that leaves out a branch can open or close a string, comment or
/// heredoc that the template's tokens do not show, which the cleanup does
/// not follow.
fn inside_rendered_tokens(
    source: &str,
    template: &Split,
    removable: impl Fn(char) -> bool,
) -> Rendered {
    let tokens = template.tokens();
    let (text, map) = rendered(source);
    let to_source = |offset: usize| {
        let i = map.partition_point(|&(at, _)| at <= offset) - 1;
        map[i].1 + (offset - map[i].0)
    };
    // Where the template's byte at `offset` lands in the rendered text, if
    // the renderer keeps it.
    let to_rendered = |offset: usize| {
        let i = map
            .partition_point(|&(_, from)| from <= offset)
            .checked_sub(1)?;
        let (at, from) = map[i];
        let len = map.get(i + 1).map_or(text.len(), |&(next, _)| next) - at;
        (offset < from + len).then(|| at + offset - from)
    };
    let mut kept = vec![false; tokens.len()];
    let rendered_split = split(&text, Mode::Jinja);
    let open_data = rendered_split.statements().last().is_some_and(|s| {
        s.data.is_some_and(|data| {
            data.format == DataFormat::Other
                && rendered_split.tokens()[data.token + 1..]
                    .iter()
                    .all(|t| t.kind() == TokenKind::Whitespace)
        })
    });
    for token in rendered_split.tokens() {
        // A line comment's trailing whitespace is not its text.
        let len = match token.kind() {
            TokenKind::Whitespace => continue,
            TokenKind::LineComment => line_comment_text(token.text(&text)).len(),
            _ => token.end() - token.start(),
        };
        let start = to_source(token.start());
        let end = to_source(token.start() + len - 1) + 1;
        let first = tokens.partition_point(|t| t.end() <= start);
        let last = tokens.partition_point(|t| t.start() < end);
        if last - first > 1 {
            kept[first..last].fill(true);
        }
    }
    let mut neighbour = vec![false; tokens.len()];
    for (i, token) in tokens.iter().enumerate() {
        let start = match token.kind() {
            TokenKind::Whitespace => token.start(),
            TokenKind::LineComment => {
                token.start() + token.text(source).trim_end_matches(&removable).len()
            }
            _ => continue,
        };
        let Some(at) = to_rendered(start) else {
            continue;
        };
        if text[..at].chars().all(is_whitespace) {
            continue;
        }
        let before = to_source(at - 1);
        let same_neighbour = before + 1 == start
            || (text[..at].ends_with(is_whitespace)
                && jinja::tags(&source[before + 1..start]).all(|t| t.kind == TagKind::Comment));
        if !same_neighbour {
            neighbour[i] = true;
        }
    }
    let inside = kept;
    let mut kept = vec![false; tokens.len()];
    // Where inline data starts and ends depends on the line breaks around
    // it, and the rendered SQL can have an `INSERT` the template's tokens do
    // not show (`{{ verb }} INTO t FORMAT CSV`). A statement with data, or
    // with a `FORMAT` and a head that can render as `INSERT`, stays as it
    // is, with the whitespace after it.
    let rendered_tokens = rendered_split.tokens();
    let word = |t: &Token, w: &str| {
        t.kind() == TokenKind::BareWord && t.text(&text).eq_ignore_ascii_case(w)
    };
    for statement in rendered_split.statements() {
        let range = statement.tokens.clone();
        let has_format = rendered_tokens[range.clone()]
            .iter()
            .any(|t| word(t, "FORMAT"));
        let head = rendered_tokens[range.start];
        let not_insert = head.kind() == TokenKind::BareWord
            && head.keyword().is_some()
            && !word(&head, "INSERT");
        let can_be_data = has_format && !not_insert;
        if statement.data.is_none() && !can_be_data {
            continue;
        }
        let after = rendered_tokens
            .get(range.end)
            .filter(|t| t.kind() == TokenKind::Whitespace)
            .map_or(range.end, |_| range.end + 1);
        let start = to_source(rendered_tokens[range.start].start());
        let end = to_source(rendered_tokens[after - 1].end() - 1) + 1;
        let first = tokens.partition_point(|t| t.end() <= start);
        let last = tokens.partition_point(|t| t.start() < end);
        kept[first..last].fill(true);
    }
    // Leaving out a branch can move where data in a format other than
    // `Values` starts or ends: a blank line or the end of the file can fall
    // elsewhere, and the rows of one branch can become another's data. The
    // rendered text with every branch taken does not show this, so from an
    // `INSERT` with a `FORMAT` after it to the end, a template with block
    // tags stays as it is: where the `FORMAT` is in the `INSERT`'s statement,
    // or a block tag between the `INSERT` and the end of its statement can
    // leave out what ends it.
    let mut open_data = open_data;
    let blocks: Vec<usize> = jinja::tags(source)
        .filter(|t| t.kind == TagKind::Block)
        .map(|t| t.range.start)
        .collect();
    if !blocks.is_empty() {
        let statements = rendered_split.statements();
        let insert = statements.iter().find_map(|statement| {
            let range = statement.tokens.clone();
            let insert = range
                .clone()
                .find(|&i| word(&rendered_tokens[i], "INSERT"))?;
            let has_format = (insert..range.end).any(|i| word(&rendered_tokens[i], "FORMAT"));
            let start = to_source(rendered_tokens[insert].start());
            let end = to_source(rendered_tokens[range.end - 1].end() - 1) + 1;
            let open_end = blocks.iter().any(|&tag| start < tag && tag < end);
            let format_later = rendered_tokens[insert..].iter().any(|t| word(t, "FORMAT"));
            (has_format || (open_end && format_later)).then_some(start)
        });
        if let Some(start) = insert {
            let first = tokens.partition_point(|t| t.end() <= start);
            kept[first..].fill(true);
            open_data = true;
        }
    }
    let data = kept;
    let mut kept: Vec<bool> = inside.iter().zip(&data).map(|(a, b)| *a || *b).collect();
    for (kept, neighbour) in kept.iter_mut().zip(neighbour) {
        *kept |= neighbour;
    }
    Rendered {
        kept,
        inside,
        data,
        open_data,
    }
}

#[cfg(test)]
mod tests;
