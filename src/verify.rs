//! Checking that formatted output is the input with only layout changed.
//!
//! [`verify`] splits both texts into statements (see [`crate::split`]),
//! which re-lexes the output, and compares their tokens with the whitespace
//! tokens left out. Every other token must have the same kind and the same
//! text, with these exceptions only:
//!
//! - a bare word the formatter recorded as re-cased (see [`Facts::recase`])
//!   may be printed in its canonical case, if it has one;
//! - a line comment may differ in the whitespace at its end, where the `\r`
//!   of a CRLF line ending sits (see [`TokenKind::LineComment`]).
//!
//! Comments, Jinja tags, strings, quoted identifiers, heredocs and `INSERT`
//! data are tokens, so their bytes must match exactly, line endings and
//! whitespace inside them included. Re-lexing the output is what catches a
//! printer that glues tokens: `-` `-1` printed as `--1` is a comment, and a
//! line comment that loses its newline swallows what follows.
//!
//! Three checks reach beyond single tokens:
//!
//! - text the formatter recorded as verbatim (see [`Facts::keep_verbatim`]),
//!   such as the left operand of `::` whose source text ClickHouse reads, must
//!   be byte-identical, whitespace included;
//! - next to a Jinja expression, whitespace between it and the
//!   neighbouring token is present in the output exactly where it is in the
//!   input (in the rendered texts, when rendering joins text, see below),
//!   since what the expression renders as could join its neighbour;
//! - in Jinja mode, when the text has a block or comment tag or a `-` marker,
//!   both texts are compared once more as the renderer would join them: with
//!   block and comment tags removed, raw blocks replaced by their text and
//!   the whitespace next to a `-` marker stripped. The rendered SQL can then
//!   hold tokens the template does not: a line break moved next to `{%-` can
//!   join a `--` comment and the next line, and `$a${% x %}` … `{% y %}$a$`,
//!   two words in the template, render as a heredoc whose text holds the
//!   whitespace between them.
//!
//! # Limitation: one render
//!
//! The rendered check models one render: the one where every `{% if %}`
//! branch is taken, and every loop body appears once. A render that leaves
//! out a branch can join text that this render keeps apart, so the verifier
//! can accept a change that alters such a render. A line comment that
//! starts before a branch runs on into whatever follows the branch when it
//! is left out: in `-- {% if x %}\nSELECT\n{% endif %} a`, a line break moved
//! in front of `a` changes the SQL when `x` is false. Likewise the end of
//! `INSERT` data at a blank line, or the start of data after a `FORMAT`, can
//! fall elsewhere in one branch. The cleanup does not make such changes to
//! data (see `crate::cleanup`); checking every branch is the job of the
//! render harness in tests.

use std::fmt;

use crate::Position;
use crate::keyword::Keyword;
use crate::lexer::Mode;
use crate::lexer::Token;
use crate::lexer::TokenKind;
use crate::lexer::is_whitespace;
use crate::lexer::jinja;
use crate::lexer::jinja::TagKind;
use crate::lexer::jinja::WhitespaceControl;
use crate::split::Split;
use crate::split::split;

/// What the formatter knows about the input that the verifier needs: where
/// it may change a word's case, and which text it must keep verbatim.
///
/// Tokens are named by their index in the input's [`Split::tokens`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// A bit per token: re-cased.
    recased: Vec<u64>,
    /// First and last token of each verbatim region.
    verbatim: Vec<(usize, usize)>,
}

impl Facts {
    /// No re-cased words and no verbatim regions.
    #[must_use]
    pub fn new() -> Facts {
        Facts::default()
    }

    /// Records that the bare word at `token` may be printed in its canonical
    /// case: upper case for a ClickHouse keyword in a keyword slot (`NULL`
    /// included), lower case for the literal words `true`, `false`, `inf`,
    /// `infinity` and `nan`. Any other case, any other word and any other
    /// kind of token stay as they are.
    pub fn recase(&mut self, token: usize) {
        let (word, bit) = (token / 64, token % 64);
        if self.recased.len() <= word {
            self.recased.resize(word + 1, 0);
        }
        self.recased[word] |= 1 << bit;
    }

    /// Whether [`recase`](Self::recase) recorded `token`.
    #[must_use]
    pub fn is_recased(&self, token: usize) -> bool {
        self.recased
            .get(token / 64)
            .is_some_and(|word| word & (1 << (token % 64)) != 0)
    }

    /// Records that the text from the start of token `first` to the end of
    /// token `last` must be printed byte for byte, whitespace included.
    /// Neither may be a whitespace token.
    pub fn keep_verbatim(&mut self, first: usize, last: usize) {
        self.verbatim.push((first.min(last), first.max(last)));
    }

    /// The regions recorded with [`keep_verbatim`](Self::keep_verbatim), as
    /// first and last token.
    #[must_use]
    pub fn verbatim_regions(&self) -> &[(usize, usize)] {
        &self.verbatim
    }
}

/// The output differs from the input in more than layout: a bug in the
/// formatter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyError {
    /// Where in the input the difference is.
    pub input: Position,
    /// Where in the output the difference is.
    pub output: Position,
    /// What differs.
    pub detail: String,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the output differs from the input at input {}, output {}: {}",
            self.input, self.output, self.detail
        )
    }
}

impl std::error::Error for VerifyError {}

/// Checks that `output` is `input` with only layout changed; see the
/// [module documentation](self).
///
/// # Errors
///
/// The first difference found.
///
/// # Panics
///
/// If either text is 4 GiB or longer, as [`split`] does.
pub fn verify(input: &str, output: &str, mode: Mode, facts: &Facts) -> Result<(), VerifyError> {
    verify_split(input, &split(input, mode), output, mode, facts)
}

/// [`verify`] for an input that has already been split: `input_split` is
/// `split(input, mode)`.
///
/// # Errors
///
/// The first difference found.
///
/// # Panics
///
/// If `output` is 4 GiB or longer, as [`split`] does.
pub fn verify_split(
    input: &str,
    input_split: &Split,
    output: &str,
    mode: Mode,
    facts: &Facts,
) -> Result<(), VerifyError> {
    let output_split = split(output, mode);
    let before = Side::new(input, input_split.tokens());
    let after = Side::new(output, output_split.tokens());
    compare(&before, &after, |i, kind, a, b| {
        same_text(kind, a, b, facts.is_recased(i))
    })?;
    for &(first, last) in &facts.verbatim {
        check_verbatim(&before, &after, first, last)?;
    }
    if mode == Mode::Jinja && render_joins_text(input) {
        check_rendered(input, output)?;
    } else {
        check_opaque_spacing(&before, &after)?;
    }
    Ok(())
}

/// One text and its tokens, with the indices of the tokens that are not
/// whitespace.
struct Side<'a> {
    text: &'a str,
    tokens: &'a [Token],
    solid: Vec<usize>,
}

impl<'a> Side<'a> {
    fn new(text: &'a str, tokens: &'a [Token]) -> Self {
        let solid = (0..tokens.len())
            .filter(|&i| tokens[i].kind() != TokenKind::Whitespace)
            .collect();
        Side {
            text,
            tokens,
            solid,
        }
    }

    /// The position of the `n`th token that is not whitespace, or of the end.
    fn position(&self, n: usize) -> Position {
        let offset = self
            .solid
            .get(n)
            .map_or(self.text.len(), |&i| self.tokens[i].start());
        Position::new(self.text, offset)
    }

    /// The start and text of the `n`th to the `m`th token that is not
    /// whitespace.
    fn span(&self, n: usize, m: usize) -> (usize, &'a str) {
        let start = self.tokens[self.solid[n]].start();
        let end = self.tokens[self.solid[m]].end();
        (start, &self.text[start..end])
    }

    fn describe(&self, n: usize) -> String {
        match self.solid.get(n) {
            Some(&i) => {
                let token = self.tokens[i];
                format!("{:?} {:?}", token.kind(), excerpt(token.text(self.text)))
            }
            None => "nothing more".to_owned(),
        }
    }
}

/// Compares the tokens that are not whitespace, pairwise; `same` decides
/// whether two tokens of one kind have equal texts, given the input token's
/// index.
fn compare(
    before: &Side<'_>,
    after: &Side<'_>,
    same: impl Fn(usize, TokenKind, &str, &str) -> bool,
) -> Result<(), VerifyError> {
    let pairs = before.solid.iter().zip(&after.solid);
    for (n, (&i, &o)) in pairs.enumerate() {
        let (a, b) = (before.tokens[i], after.tokens[o]);
        if a.kind() != b.kind() || !same(i, a.kind(), a.text(before.text), b.text(after.text)) {
            return Err(mismatch(before, after, n));
        }
    }
    if before.solid.len() == after.solid.len() {
        Ok(())
    } else {
        Err(mismatch(
            before,
            after,
            before.solid.len().min(after.solid.len()),
        ))
    }
}

fn mismatch(before: &Side<'_>, after: &Side<'_>, n: usize) -> VerifyError {
    VerifyError {
        input: before.position(n),
        output: after.position(n),
        detail: format!(
            "token {} is {} in the input and {} in the output",
            n + 1,
            before.describe(n),
            after.describe(n)
        ),
    }
}

fn same_text(kind: TokenKind, a: &str, b: &str, recased: bool) -> bool {
    match kind {
        TokenKind::LineComment => line_comment_text(a) == line_comment_text(b),
        TokenKind::BareWord if recased => a == b || canonical_case(a).is_some_and(|c| c == b),
        _ => a == b,
    }
}

/// The case a re-cased word is printed in, if it may be re-cased at all: a
/// literal word (`true`, `false`, `inf`, `infinity`, `nan`) in lower case,
/// and any other ClickHouse keyword, `NULL` among them, in upper case.
fn canonical_case(word: &str) -> Option<String> {
    let lower = word.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "true" | "false" | "inf" | "infinity" | "nan"
    ) {
        Some(lower)
    } else if Keyword::from_word(word).is_some() {
        Some(word.to_ascii_uppercase())
    } else {
        None
    }
}

/// A line comment's text without the whitespace at its end, by the lexer's
/// definition of whitespace: once the comment's line ends, that whitespace is
/// the same as the whitespace token after it.
pub(crate) fn line_comment_text(comment: &str) -> &str {
    comment.trim_end_matches(is_whitespace)
}

fn check_verbatim(
    before: &Side<'_>,
    after: &Side<'_>,
    first: usize,
    last: usize,
) -> Result<(), VerifyError> {
    let (Ok(n), Ok(m)) = (
        before.solid.binary_search(&first),
        before.solid.binary_search(&last),
    ) else {
        return Err(VerifyError {
            input: Position::new(before.text, before.text.len()),
            output: Position::new(after.text, after.text.len()),
            detail: format!(
                "verbatim region {first}..={last} does not start and end at tokens \
                 other than whitespace"
            ),
        });
    };
    let (start, text) = before.span(n, m);
    let (out_start, out_text) = after.span(n, m);
    if text == out_text {
        return Ok(());
    }
    let at = text
        .bytes()
        .zip(out_text.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    Err(VerifyError {
        input: Position::new(before.text, start + at),
        output: Position::new(after.text, out_start + at),
        detail: format!(
            "the verbatim text {:?} is printed as {:?}",
            excerpt(text),
            excerpt(out_text)
        ),
    })
}

/// Between an expression and the token on either side,
/// whitespace is present in the output exactly where it is in the input: a
/// `{{ … }}` can render as anything, so `- {{ x }}` printed as `-{{ x }}`
/// turns into a comment when `x` renders as `-1`.
///
/// Where rendering joins text, this runs on the rendered texts, where block
/// tags and stripped whitespace are gone and raw blocks are text, so the
/// neighbours are the ones the SQL ends up with.
fn check_opaque_spacing(before: &Side<'_>, after: &Side<'_>) -> Result<(), VerifyError> {
    let opaque = |n: usize| {
        matches!(
            before.tokens[before.solid[n]].kind(),
            TokenKind::JinjaExpr | TokenKind::Verbatim
        )
    };
    let spaced = |side: &Side<'_>, n: usize| side.solid[n + 1] > side.solid[n] + 1;
    for n in 0..before.solid.len().saturating_sub(1) {
        if (opaque(n) || opaque(n + 1)) && spaced(before, n) != spaced(after, n) {
            return Err(VerifyError {
                input: before.position(n + 1),
                output: after.position(n + 1),
                detail: format!(
                    "{} and {} are {} in the output, and one of them renders as anything",
                    before.describe(n),
                    before.describe(n + 1),
                    if spaced(before, n) {
                        "glued"
                    } else {
                        "separated"
                    }
                ),
            });
        }
    }
    Ok(())
}

/// Whether rendering the template can join its text differently from how
/// its tokens lie: it has a block or comment tag, which renders nothing, or
/// a `-` marker, which strips whitespace.
pub(crate) fn render_joins_text(text: &str) -> bool {
    ["{%", "{#", "{{-", "-}}"]
        .iter()
        .any(|marker| text.contains(marker))
}

/// The text as the renderer joins it: block and comment tags removed, the
/// whitespace a `-` marker strips removed, and raw blocks replaced by the
/// text they hold. Expressions stay as they are, standing for whatever they
/// render.
///
/// The renderer knows nothing of SQL, so this works on the text, not the
/// tokens: a tag inside a comment or string strips whitespace too.
///
/// The second value maps the result back to `text`: for each piece copied,
/// in order, its offset in the result and in `text`.
pub(crate) fn rendered(text: &str) -> (String, Vec<(usize, usize)>) {
    let mut out = String::with_capacity(text.len());
    let mut map = Vec::new();
    let mut copy = |out: &mut String, from: usize, to: usize| {
        if from < to {
            map.push((out.len(), from));
            out.push_str(&text[from..to]);
        }
    };
    let mut pos = 0;
    let mut strip_next = false;
    for tag in jinja::tags(text) {
        let (before, after) = tag.whitespace_control(text);
        let (mut from, mut to) = (pos, tag.range.start);
        if strip_next {
            from = to - text[from..to].trim_start().len();
        }
        if before == WhitespaceControl::Strip {
            to = from + text[from..to].trim_end().len();
        }
        copy(&mut out, from, to);
        match tag.kind {
            TagKind::Expr => copy(&mut out, tag.range.start, tag.range.end),
            TagKind::Raw => {
                let (from, to) = raw_text(text, tag.range.clone());
                copy(&mut out, from, to);
            }
            TagKind::Block | TagKind::Comment => {}
        }
        strip_next = after == WhitespaceControl::Strip;
        pos = tag.range.end;
    }
    let mut from = pos;
    if strip_next {
        from = text.len() - text[from..].trim_start().len();
    }
    copy(&mut out, from, text.len());
    (out, map)
}

/// The text a closed raw block renders as, as a range of `text`: what lies
/// between `{% raw %}` and `{% endraw %}`, less the whitespace that the
/// inner markers (`{% raw -%}`, `{%- endraw %}`) strip. An unclosed raw block
/// renders nothing.
fn raw_text(text: &str, block: std::ops::Range<usize>) -> (usize, usize) {
    let inner = &text[block.clone()];
    let (Some(open_end), Some(close_start)) = (inner.find("%}"), inner.rfind("{%")) else {
        return (block.start, block.start);
    };
    if open_end + 2 > close_start {
        return (block.start, block.start);
    }
    let mut from = block.start + open_end + 2;
    let mut to = block.start + close_start;
    if inner[..open_end].ends_with('-') {
        from = to - text[from..to].trim_start().len();
    }
    if inner[close_start + 2..].starts_with('-') {
        to = from + text[from..to].trim_end().len();
    }
    (from, to)
}

/// Compares the two texts as rendered with every branch taken; see the
/// [limitation](self#limitation-one-render) this leaves.
fn check_rendered(input: &str, output: &str) -> Result<(), VerifyError> {
    let input_rendered = rendered(input).0;
    let output_rendered = rendered(output).0;
    let input_split = split(&input_rendered, Mode::Jinja);
    let output_split = split(&output_rendered, Mode::Jinja);
    let mut before = Side::new(&input_rendered, input_split.tokens());
    let mut after = Side::new(&output_rendered, output_split.tokens());
    // An error token runs to the end of the text and takes in whitespace the
    // template's tokens kept apart, so only its kind can be compared, and
    // nothing after it.
    if let Some(n) = before
        .solid
        .iter()
        .position(|&i| before.tokens[i].kind().is_error())
    {
        before.solid.truncate(n + 1);
        after.solid.truncate(n + 1);
    }
    // Re-cased words were checked against the template; here only the
    // joining matters.
    compare(&before, &after, |_, kind, a, b| {
        kind.is_error() || same_text(kind, a, b, kind == TokenKind::BareWord)
    })
    .and_then(|()| check_opaque_spacing(&before, &after))
    .map_err(|e| VerifyError {
        input: Position::new(input, 0),
        output: Position::new(output, 0),
        detail: format!(
            "rendered, with block tags removed and `-` markers applied, {} (at {} and {} \
             of the rendered texts)",
            e.detail, e.input, e.output
        ),
    })
}

/// The start of a token's text, for messages.
fn excerpt(text: &str) -> String {
    const MAX: usize = 40;
    match text.char_indices().nth(MAX) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests;
