//! `chairlift` — a formatter for ClickHouse SQL, including SQL files that
//! contain Jinja templates, such as dbt models.
//!
//! [`format()`] lays a text out in chairlift's style, and [`check`] says
//! whether a text is formatted already. The style has two settings, both in
//! [`Options`]: the line width, 100 by default, and whether the text is a
//! Jinja template. The repository's
//! [README](https://github.com/glennib/chairlift#readme) describes the style
//! and the `chairlift` command line.
//!
//! ```
//! use chairlift::Options;
//!
//! let options = Options::default();
//! let formatted = chairlift::format(
//!     "select a, b from t where x = 1;  \r\n\n\n\nsystem flush logs;",
//!     &options,
//! )?;
//! assert_eq!(
//!     formatted,
//!     "SELECT\n    a,\n    b\nFROM t\nWHERE x = 1;\n\n\nsystem flush logs;\n",
//! );
//! assert!(chairlift::check(&formatted, &options)?);
//!
//! let template = Options::default().with_jinja(true);
//! let formatted = chairlift::format("select {{ col }} from {{ ref('t') }}", &template)?;
//! assert_eq!(formatted, "SELECT {{ col }}\nFROM {{ ref('t') }}\n");
//! # Ok::<(), chairlift::Error>(())
//! ```
//!
//! # What formatting changes
//!
//! Only whitespace and the case of keywords. A word is upper cased only where
//! the parser read a keyword, so a column named `date` keeps its case, and
//! `true`, `false`, `inf` and `nan` are written in lower case, as
//! `clickhouse-format` prints them. No token is added, removed, reordered or
//! rewritten: string and number literals, comments, Jinja tags and the data
//! of an `INSERT` keep every byte, and names (identifiers, aliases, and
//! function, type, engine and setting names) keep their case.
//!
//! [`format()`] checks this on every call. The result must hold every token
//! of the input once and in order, and the [verifier](verify) lexes it again
//! and compares its tokens with those of the input, so that a line comment
//! that would swallow the next line, or two tokens that would run together,
//! are caught. A result that fails is a bug, and [`format()`] returns an
//! error in its place. It also returns an error, and no text, for input with
//! a lexer error, such as an unclosed string, and for a template it refuses;
//! its documentation lists the cases.
//!
//! # Statements
//!
//! The [parser](syntax) turns each statement into a lossless syntax tree.
//! A statement of these kinds is then [laid out](layout) by the rule for its
//! kind:
//!
//! - `SELECT` queries, with `WITH` and set operations such as `UNION`;
//! - `INSERT`;
//! - `CREATE TABLE`, `VIEW`, `MATERIALIZED VIEW`, `DICTIONARY`, `DATABASE`,
//!   `FUNCTION` and `INDEX`;
//! - `ALTER` of tables and databases, and the lightweight `DELETE` and
//!   `UPDATE`;
//! - `GRANT`, `REVOKE`, `SET ROLE`, and the statements that create, alter
//!   and drop users, roles, quotas, row policies and settings profiles;
//! - `DROP`, `DETACH`, `UNDROP`, `TRUNCATE`, `RENAME`, `EXCHANGE`,
//!   `OPTIMIZE`, `SET` and `USE`;
//! - `SHOW` and `DESCRIBE`;
//! - `ATTACH`: a table's columns and clauses after `FROM 'path'` are laid
//!   out as `CREATE TABLE` lays them out, and any other definition after the
//!   name is kept as written.
//!
//! Every other statement is kept as written: `SYSTEM`, `EXPLAIN`, KQL
//! `PRINT`, the rarer kinds the parser does not know, such as `KILL`,
//! `CHECK` and `BACKUP`, statements joined by `PARALLEL WITH`, and anything
//! it cannot parse. In a statement kept as written, trailing whitespace goes
//! and runs of blank lines are shortened, and nothing else changes.
//! [`format_with_statements`] says what became of each statement, and why
//! one was kept.
//!
//! The [layout] turns the tree into a [document](doc), whose printer chooses
//! the line breaks.
//!
//! # Jinja templates
//!
//! With [`Options::jinja`] set, the [lexer] reads `{{ … }}`, `{% … %}` and
//! `{# … #}` as template tags, and the text inside a tag is never changed.
//! The parser steps over block tags, so it sees the SQL of every branch at
//! once, and the layout puts each block tag on a line of its own. Where the
//! layout cannot be sure that every render of the template keeps its
//! meaning, such as where a tag stands for a keyword or a render could join
//! two tokens into one, the statement is kept as written. A template in
//! which a line comment can run on past its line break in a render is
//! refused with [`Error::Template`]: moving a line break there could change
//! what the comment comments out.
//!
//! # Features
//!
//! - `cli`, on by default: the `chairlift` binary. It enables `oracle` and
//!   the binary's own dependencies; the library needs none of them, so a
//!   crate that uses only the library can turn default features off.
//! - `oracle`: the `chairlift::oracle` module, which runs ClickHouse's
//!   `clickhouse-format` on a text and its formatted output to check that
//!   ClickHouse reads both the same, for the binary's `--verify`.
//! - `template`: the `chairlift::template` module, a test harness that
//!   renders a template once per combination of its branches and loop
//!   counts, and checks that formatting kept the SQL of every render. It
//!   adds minijinja.
//! - `corpus-tool`: the `corpus` binary, a development tool that formats,
//!   verifies and checks whole directory trees.
//!
//! # ClickHouse version
//!
//! The grammar, the keyword list and the lexer rules follow one ClickHouse
//! release, [`CLICKHOUSE_VERSION`]. [`KEYWORDS_VERSION`] is the release the
//! keyword list was generated from, and so the release whose keywords are
//! re-cased.

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
/// as written: a statement the parser does not know or could not parse, or
/// one holding a Jinja shape the rules do not cover. A statement kept as
/// written gets the safe cleanup only:
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
/// whitespace too. A template that a render can turn into a string, heredoc
/// or comment around whitespace between its tokens is returned unchanged.
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
    let output = if layout.unchanged {
        source.to_owned()
    } else {
        layout.doc.print(&doc::PrintOptions::from(options))
    };
    if too_large(output.len()) {
        return Err(Error::TooLarge { len: output.len() });
    }
    verify::verify_split(source, &split, &output, mode, parse.facts())?;
    Ok((output, layout.statements))
}

/// Whether `source` is formatted already: whether [`format()`] would return it
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
    /// The line, counting from 1; lines end at `\n`, `\r\n` or a lone `\r`,
    /// as the formatter ends them.
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
        let line_start = before.rfind(['\n', '\r']).map_or(0, |i| i + 1);
        // A `\n` after a `\r` ends the line the `\r` already ended.
        let breaks = before
            .bytes()
            .zip(std::iter::once(0).chain(before.bytes()))
            .filter(|&(b, previous)| b == b'\r' || (b == b'\n' && previous != b'\r'))
            .count();
        Position {
            offset,
            line: breaks + 1,
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

/// The ClickHouse release whose parser keywords chairlift knows: the release
/// [`keyword::Keyword`] was generated from. A word re-cased in a keyword slot
/// is a keyword of this release.
///
/// `scripts/generate-keywords.sh` writes it with the list and refuses a
/// ClickHouse checkout of another release than [`CLICKHOUSE_VERSION`].
///
/// ```
/// assert_eq!(chairlift::KEYWORDS_VERSION, chairlift::CLICKHOUSE_VERSION);
/// ```
pub const KEYWORDS_VERSION: &str = keyword::Keyword::SOURCE_VERSION;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_lines_as_the_formatter_ends_them() {
        let at = |text: &str, offset| {
            let p = Position::new(text, offset);
            (p.line, p.column)
        };
        assert_eq!(at("ab\ncd", 4), (2, 2));
        assert_eq!(at("ab\r\ncd", 5), (2, 2));
        assert_eq!(at("ab\rcd", 4), (2, 2));
        assert_eq!(at("a\r\rb\n\nc", 6), (5, 1));
        // Columns count `char`s.
        assert_eq!(at("'æøå', `x", 10), (1, 8));
    }

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
