//! `chairlift` — a formatter for ClickHouse SQL, including SQL files that
//! contain Jinja templates.
//!
//! chairlift changes only whitespace and the case of keywords; it never adds,
//! removes, reorders or rewrites tokens. The library has no formatting API yet:
//! the crate is pre-release, and `docs/MILESTONES.md` in the repository lays
//! out what comes next.

pub mod keyword;

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
