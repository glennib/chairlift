//! `chairlift` — a formatter for ClickHouse SQL, including SQL files that
//! contain Jinja templates.
//!
//! chairlift changes only whitespace and the case of keywords; it never adds,
//! removes, reorders or rewrites tokens. The library has no formatting API yet:
//! the crate is pre-release, and `docs/MILESTONES.md` in the repository lays
//! out what comes next.

pub mod keyword;
pub mod lexer;
pub mod split;
pub mod verify;

use std::fmt;

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
