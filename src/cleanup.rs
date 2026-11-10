//! The safe cleanup: changes to whitespace that cannot change what any
//! token is.
//!
//! - Whitespace at the end of a line is removed, and so is the `\r` of a
//!   CRLF line ending: only the `\n` and the indentation of the next line are
//!   kept of a whitespace token that ends a line. A line comment loses the
//!   whitespace at its end, `\r` included (see [`TokenKind::LineComment`]),
//!   except that `# ` keeps its space, without which it is not a comment.
//!   In a template, characters that ClickHouse counts as whitespace and the
//!   renderer does not (U+FEFF, U+200B and a few more) stay, because they
//!   stop a `-` marker from stripping what lies beyond them.
//! - In a template whose rendering joins text (see
//!   [`render_joins_text`]), whitespace changes only where the rendered SQL
//!   puts it between the same neighbours as the template does; the rule and
//!   why it is enough are at [`inside_rendered_tokens`].
//! - Blank lines collapse: at most two between statements, at most one
//!   inside a statement, and none right after `(` or before `)`.
//! - The file starts without a blank line (a byte-order mark stays) and ends
//!   with exactly one `\n`; a file of only whitespace becomes empty.
//!   ClickHouse lexes the first and last bytes of the input specially (`.`
//!   at the very start begins a number, `1e` at the very end is a bare word),
//!   so where removing or adding whitespace there would change the first or
//!   last token, it is left as it is.
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

use crate::lexer::Mode;
use crate::lexer::TokenKind;
use crate::lexer::is_whitespace;
use crate::lexer::jinja;
use crate::lexer::jinja::TagKind;
use crate::lexer::same_before_newline;
use crate::split::DataFormat;
use crate::split::Split;
use crate::split::split;
use crate::verify::line_comment_text;
use crate::verify::render_joins_text;
use crate::verify::rendered;

/// The cleaned-up text of `source`, which `split` came from.
pub(crate) fn cleanup(source: &str, split: &Split, mode: Mode) -> String {
    let tokens = split.tokens();
    let Some(last_solid) = tokens
        .iter()
        .rposition(|t| t.kind() != TokenKind::Whitespace)
    else {
        return String::new();
    };
    // ClickHouse looks at most two bytes past a token, so whether the last
    // tokens lex the same with a newline after them depends on the last three.
    let tail = (0..=last_solid)
        .rev()
        .filter(|&i| tokens[i].kind() != TokenKind::Whitespace)
        .take(3)
        .last()
        .unwrap_or(last_solid);
    // A template's `-` markers strip what the renderer counts as whitespace,
    // which leaves out some of ClickHouse's (U+FEFF, U+200B and others); such
    // a character stops the stripping, so in a template it stays.
    let removable = |c: char| is_whitespace(c) && (mode == Mode::Sql || c.is_whitespace());
    let Rendered {
        kept,
        open_data: rendered_open_data,
    } = if mode == Mode::Jinja && render_joins_text(source) {
        inside_rendered_tokens(source, split, removable)
    } else {
        Rendered::default()
    };
    // Whether the part of a whitespace token up to its last line break, which
    // the cleanup rewrites, is removable; a leading byte-order mark stays.
    let lines_removable = |text: &str, first: bool| {
        let text = if first {
            text.strip_prefix('\u{feff}').unwrap_or(text)
        } else {
            text
        };
        let end = text.rfind('\n').map_or(0, |i| i + 1);
        text[..end].chars().all(|c| c == '\n' || removable(c))
    };
    let dot_first = tokens[1..]
        .iter()
        .find(|t| {
            !matches!(
                t.kind(),
                TokenKind::JinjaBlock | TokenKind::JinjaComment | TokenKind::Whitespace
            )
        })
        .is_some_and(|t| t.kind() == TokenKind::Dot);
    let mut tail_start = 0;
    let mut out = String::with_capacity(source.len() + 1);
    let statements = split.statements();
    let mut statement = 0;
    let mut prev_solid = None;
    for (i, token) in tokens.iter().enumerate().take(last_solid + 1) {
        if i == tail {
            tail_start = out.len();
        }
        let text = token.text(source);
        match token.kind() {
            _ if kept.get(i).copied().unwrap_or(false) => out.push_str(text),
            // At the very start of the input `.` begins a number, so the
            // whitespace in front of a `Dot` stays, also where only tags that
            // render nothing come between, and whitespace that a `-` marker
            // may strip.
            TokenKind::Whitespace if i == 0 && dot_first => out.push_str(text),
            TokenKind::Whitespace if !lines_removable(text, i == 0) => out.push_str(text),
            TokenKind::Whitespace if i == 0 => {
                let text = match text.strip_prefix('\u{feff}') {
                    Some(rest) => {
                        out.push('\u{feff}');
                        rest
                    }
                    None => text,
                };
                out.push_str(after_last_newline(text));
            }
            TokenKind::Whitespace => {
                let newlines = text.bytes().filter(|&b| b == b'\n').count();
                if newlines == 0 {
                    out.push_str(text);
                    continue;
                }
                while statements.get(statement).is_some_and(|s| s.tokens.end <= i) {
                    statement += 1;
                }
                let inside = statements
                    .get(statement)
                    .is_some_and(|s| s.tokens.start < i);
                let next = tokens[i + 1].kind();
                let max = if !inside {
                    3
                } else if prev_solid == Some(TokenKind::OpeningRoundBracket)
                    || next == TokenKind::ClosingRoundBracket
                {
                    1
                } else {
                    2
                };
                for _ in 0..newlines.min(max) {
                    out.push('\n');
                }
                out.push_str(after_last_newline(text));
            }
            TokenKind::LineComment => {
                let trimmed = text.trim_end_matches(removable);
                out.push_str(if trimmed == "#" { text } else { trimmed });
            }
            _ => out.push_str(text),
        }
        if token.kind() != TokenKind::Whitespace {
            prev_solid = Some(token.kind());
        }
    }
    let last = tokens[last_solid];
    let open_data = statements
        .last()
        .and_then(|s| s.data)
        .is_some_and(|data| data.token == last_solid && data.format == DataFormat::Other);
    if rendered_open_data {
        // The rendered text ends in data that runs to its end or to a blank
        // line; whatever follows the last token is data or ends the data.
        out.push_str(&source[last.end()..]);
    } else if !open_data {
        // Tokens followed by whitespace in the input lex the same before the
        // final newline. Tokens that end the input may lex differently with
        // a newline after them, in the template or in its rendered text;
        // then the end stays as it is.
        let text = last.text(source);
        let followed = last.end() < source.len()
            || (last.kind() == TokenKind::LineComment && line_comment_text(text) != text);
        let prev_significant = tokens[..tail]
            .iter()
            .rev()
            .find(|t| t.kind().is_significant())
            .map(|t| t.kind());
        let tail_same =
            same_before_newline(mode, prev_significant, &out[tail_start..], tail_start > 0);
        // Where the tail lands in the rendered text, and what comes before
        // it there, only the whole rendered text tells.
        let rendered_same = || {
            !render_joins_text(&out)
                || same_before_newline(Mode::Jinja, None, &rendered(&out).0, false)
        };
        if followed || (tail_same && (mode == Mode::Sql || rendered_same())) {
            out.push('\n');
        }
    } else if last.end() < source.len() {
        // The data ended at a blank line, which must stay.
        out.push_str("\n\n");
    }
    out
}

/// What a template's rendered text means for the cleanup.
#[derive(Default)]
struct Rendered {
    /// For each token of the template, whether it must stay as it is.
    kept: Vec<bool>,
    /// The rendered text ends in `INSERT` data in a format other than
    /// `Values`, with nothing but whitespace after it.
    open_data: bool,
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
            kept[i] = true;
        }
    }
    // Where inline data starts and ends depends on the line breaks around
    // it, and the rendered SQL can have an `INSERT` the template's tokens do
    // not show. Its statement stays as it is, with the whitespace after it.
    let rendered_tokens = rendered_split.tokens();
    for statement in rendered_split.statements() {
        let range = statement.tokens.clone();
        let has_format = rendered_tokens[range.clone()].iter().any(|t| {
            t.kind() == TokenKind::BareWord && t.text(&text).eq_ignore_ascii_case("FORMAT")
        });
        if statement.data.is_none() && !has_format {
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
    Rendered { kept, open_data }
}

/// The indentation part of a whitespace token: what follows its last `\n`.
fn after_last_newline(text: &str) -> &str {
    text.rfind('\n').map_or(text, |i| &text[i + 1..])
}

#[cfg(test)]
mod tests;
