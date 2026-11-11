//! `chairlift` — a formatter for ClickHouse SQL, including SQL files that
//! contain Jinja templates.
//!
//! chairlift changes only whitespace and the case of keywords; it never adds,
//! removes, reorders or rewrites tokens, and [`format`] checks every result
//! against its input with the [verifier](verify) before returning it.
//!
//! The crate is pre-release: [`format`](fn@format) lays out `SELECT` and
//! `WITH` queries, `CREATE TABLE`, `CREATE MATERIALIZED VIEW`, `INSERT`, and
//! `DROP`, `TRUNCATE`, `ATTACH`, `RENAME`, `OPTIMIZE`, `SET`, `USE` and their
//! kin, and keeps every other statement as written, with only the safe
//! cleanup of its whitespace; `docs/MILESTONES.md` in the repository lays out what comes
//! next. The [parser](syntax) turns the text into a lossless syntax tree, the
//! [layout] turns that into a [document](doc), and the document's printer
//! chooses the line breaks.
//!
//! ```
//! let options = chairlift::Options::default();
//! let formatted = chairlift::format("select a, b from t where x = 1;  \r\n\n\n\nsystem flush logs;", &options)?;
//! assert_eq!(formatted, "SELECT\n    a,\n    b\nFROM t\nWHERE x = 1;\n\n\nsystem flush logs;\n");
//! assert!(chairlift::check(&formatted, &options)?);
//! # Ok::<(), chairlift::Error>(())
//! ```

mod cleanup;
pub mod doc;
mod error;
pub mod keyword;
pub mod layout;
pub mod lexer;
mod options;
#[cfg(feature = "oracle")]
pub mod oracle;
pub mod split;
pub mod syntax;
#[cfg(feature = "template")]
pub mod template;
pub mod verify;

use std::fmt;

pub use error::CommentEnd;
pub use error::Error;
pub use error::LexError;
pub use error::TemplateError;
pub use options::Options;

use crate::split::split;

/// Formats `source`.
///
/// Every statement is [parsed](syntax) and either [laid out](layout) by the
/// style's rules (keywords upper case, clauses one per line, lists that fit
/// on a line or go one item per line, at [`Options::line_width`]), or kept
/// as written: a statement the layout has no rule for yet, or one holding a
/// Jinja shape the rules do not cover. A statement kept as written gets the
/// safe cleanup only:
///
/// - whitespace at the end of lines goes, and CRLF line endings become LF;
/// - blank lines collapse to one at most, with none right after `(` or
///   before `)`.
///
/// Between statements, at most two blank lines are kept, and there is one
/// before and after a statement that spans lines. The text starts without a
/// blank line and ends with exactly one newline. Nothing inside a token
/// changes: comments, strings, heredocs, Jinja tags and `INSERT` data keep
/// every byte, line endings included. Data in a format other than `Values`
/// that runs to the end of the text gets no final newline, and such data that
/// ends at a blank line keeps it. Text the parser finds ClickHouse reads as
/// written, such as the left operand of `[1,2]::Array(UInt8)`, keeps its
/// whitespace too.
///
/// The result is checked to hold every token of `source` once, in order,
/// and [verified](verify) against `source` before it is returned.
///
/// # Errors
///
/// - [`Error::Lex`] if `source` has a lexer error, such as an unclosed string
///   or comment: what its tokens are after that point cannot be relied on.
/// - [`Error::Verify`] if the result failed verification, and
///   [`Error::Layout`] if the layout lost or repeated a token: bugs, for
///   which no output is returned.
/// - [`Error::Template`] if `source` is a template in which a line comment
///   can run on past its line break in a render formatting could change:
///   such a text is left as it is.
/// - [`Error::TooLarge`] if `source` is 4 GiB or longer.
pub fn format(source: &str, options: &Options) -> Result<String, Error> {
    format_with_statements(source, options).map(|(output, _)| output)
}

/// [`format()`], and what the layout did with each statement of `source`:
/// whether it laid it out, or kept it as written and why.
///
/// # Errors
///
/// As [`format()`].
pub fn format_with_statements(
    source: &str,
    options: &Options,
) -> Result<(String, Vec<layout::Statement>), Error> {
    let too_large = |len: usize| u32::try_from(len).is_err();
    if too_large(source.len()) {
        return Err(Error::TooLarge { len: source.len() });
    }
    let mode = options.mode();
    let split = split(source, mode);
    if let Some(token) = split.first_error() {
        return Err(LexError {
            position: Position::new(source, token.start()),
            kind: token.kind(),
        }
        .into());
    }
    if mode == lexer::Mode::Jinja
        && let Some((comment, reason)) = layout::comment_ended_by_rendering(source, split.tokens())
    {
        return Err(TemplateError {
            position: Position::new(source, comment.start()),
            reason,
        }
        .into());
    }
    let parse = syntax::parse_split(source, &split, syntax::Entry::File);
    let layout = layout::layout(source, &split, &parse, mode);
    layout.doc.check_tokens()?;
    let output = layout.doc.print(&doc::PrintOptions::from(options));
    if too_large(output.len()) {
        return Err(Error::TooLarge { len: output.len() });
    }
    verify::verify_split(source, &split, &output, mode, parse.facts())?;
    Ok((output, layout.statements))
}

/// Whether `source` is formatted already: whether [`format`] would return it
/// unchanged.
///
/// # Errors
///
/// As [`format()`].
pub fn check(source: &str, options: &Options) -> Result<bool, Error> {
    Ok(format(source, options)? == source)
}

/// A place in a text: a byte offset, and the line and column it is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    /// The byte offset.
    pub offset: usize,
    /// The line, counting from 1; lines end at `\n`.
    pub line: usize,
    /// The column, counting from 1, in `char`s.
    pub column: usize,
}

impl Position {
    /// The position of byte `offset` in `text`, or of the end of `text` if
    /// `offset` is past it or not on a `char` boundary.
    #[must_use]
    pub fn new(text: &str, offset: usize) -> Position {
        let offset = if text.is_char_boundary(offset) {
            offset
        } else {
            text.len()
        };
        let before = &text[..offset];
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        Position {
            offset,
            line: before.bytes().filter(|&b| b == b'\n').count() + 1,
            column: before[line_start..].chars().count() + 1,
        }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// The ClickHouse release whose grammar chairlift targets.
///
/// Keyword lists and lexer rules are taken from this release's source, and the
/// tests use its `clickhouse-format` as an oracle.
///
/// ```
/// assert!(chairlift::CLICKHOUSE_VERSION.ends_with("-stable"));
/// ```
pub const CLICKHOUSE_VERSION: &str = "26.10.2.103-stable";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clickhouse_version_has_four_numeric_parts() {
        let (numbers, channel) = CLICKHOUSE_VERSION
            .split_once('-')
            .expect("version has a release channel suffix");
        assert_eq!(channel, "stable");
        let parts: Vec<&str> = numbers.split('.').collect();
        assert_eq!(parts.len(), 4);
        assert!(
            parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        );
    }
}
