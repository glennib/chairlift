//! Statistics over a parsed statement: how much of it parsed.

use rowan::NodeOrToken;

use crate::syntax::FallbackLevel;
use crate::syntax::Role;
use crate::syntax::SyntaxKind;
use crate::syntax::SyntaxNode;

/// What became of one statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatementStats {
    /// The statement's kind, from its leading words, in lower case:
    /// `select`, `with`, `create table`, `create materialized view`,
    /// `insert`, `drop table`, …: the first word, refined by the object
    /// kind for `CREATE`, `DROP`, `ALTER` and the like.
    pub kind: String,
    /// Its byte range in the text, `;` included.
    pub range: std::ops::Range<usize>,
    /// How much of it parsed.
    pub outcome: StatementOutcome,
    /// Its clauses, parsed or verbatim.
    pub clauses: usize,
    /// Its clauses kept verbatim.
    pub verbatim_clauses: usize,
    /// Its list items, at any depth, parsed or verbatim.
    pub items: usize,
    /// Its list items kept verbatim.
    pub verbatim_items: usize,
    /// Its regions kept as written by design, such as `OVER (…)`: its
    /// [`Opaque`](SyntaxKind::Opaque) nodes.
    pub opaque: usize,
}

/// How much of a statement parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatementOutcome {
    /// Only a `;`.
    Empty,
    /// Parsed whole: no fallback, and nothing kept as written.
    Parsed,
    /// Parsed with no fallback, but with regions kept as written by design
    /// ([`Opaque`](SyntaxKind::Opaque) nodes, such as `OVER (…)`), which the
    /// formatter leaves alone.
    Opaque,
    /// Parsed, with some items or clauses kept verbatim by a fallback.
    Partial,
    /// Kept verbatim as a whole: a kind the parser does not know, or one
    /// that did not parse.
    Verbatim,
}

impl StatementOutcome {
    /// A short name, for tables and CSV.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            StatementOutcome::Empty => "empty",
            StatementOutcome::Parsed => "parsed",
            StatementOutcome::Opaque => "opaque",
            StatementOutcome::Partial => "partial",
            StatementOutcome::Verbatim => "verbatim",
        }
    }
}

pub(crate) fn statement_stats(statement: &SyntaxNode) -> StatementStats {
    let range = statement.text_range();
    let body = statement.first_child();
    let mut stats = StatementStats {
        kind: statement_label(statement),
        range: usize::from(range.start())..usize::from(range.end()),
        outcome: StatementOutcome::Empty,
        clauses: 0,
        verbatim_clauses: 0,
        items: 0,
        verbatim_items: 0,
        opaque: 0,
    };
    let Some(body) = body else {
        return stats;
    };
    let mut fallback = false;
    for node in body.descendants() {
        let kind = node.kind();
        let in_list = node.parent().is_some_and(|p| p.kind().is_list());
        if in_list || kind == SyntaxKind::VerbatimItem {
            stats.items += 1;
        }
        if kind.is_clause() {
            stats.clauses += 1;
        }
        match kind.role() {
            Role::Fallback(FallbackLevel::Item) => {
                stats.verbatim_items += 1;
                fallback = true;
            }
            Role::Fallback(FallbackLevel::Clause) => {
                stats.verbatim_clauses += 1;
                fallback = true;
            }
            Role::Fallback(FallbackLevel::Statement) => fallback = true,
            Role::Opaque => stats.opaque += 1,
            _ => {}
        }
    }
    stats.outcome = if body.kind() == SyntaxKind::VerbatimStatement {
        StatementOutcome::Verbatim
    } else if fallback {
        StatementOutcome::Partial
    } else if stats.opaque > 0 {
        StatementOutcome::Opaque
    } else {
        StatementOutcome::Parsed
    };
    stats
}

/// A statement's kind, from its first words, in lower case.
///
/// The first word names it (`select`, `insert`, `set`, …), with these
/// refinements: a statement that starts with `(` is a `select`;
/// `CREATE`, `ATTACH` and `REPLACE` take the object kind after them, skipping
/// `OR REPLACE` and `TEMPORARY` (`create table`, `create materialized view`,
/// `create view`, `create dictionary`); `DROP`, `ALTER`, `DETACH`, `SHOW`,
/// `EXISTS`, `TRUNCATE`, `UNDROP` and `SYSTEM` take the word after them
/// (`drop table`, `alter table`, `show create`).
#[must_use]
pub(crate) fn statement_label(statement: &SyntaxNode) -> String {
    let mut words = statement
        .descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
        .filter(|t| !t.kind().is_trivia())
        .take(6)
        .map(|t| match t.kind() {
            SyntaxKind::BareWord | SyntaxKind::Keyword | SyntaxKind::LiteralWord => {
                t.text().to_ascii_lowercase()
            }
            SyntaxKind::OpeningRoundBracket => "(".to_owned(),
            SyntaxKind::JinjaExpr => "{{".to_owned(),
            SyntaxKind::Semicolon => String::new(),
            kind => kind.name().to_ascii_lowercase(),
        })
        .filter(|w| !w.is_empty());
    let Some(first) = words.next() else {
        return "empty".to_owned();
    };
    let rest: Vec<String> = words.collect();
    let word = |i: usize| rest.get(i).map_or("", String::as_str);
    match first.as_str() {
        "(" => "select".to_owned(),
        "create" | "attach" | "replace" => {
            let mut i = 0;
            if word(i) == "or" && word(i + 1) == "replace" {
                i += 2;
            }
            if word(i) == "temporary" {
                i += 1;
            }
            let object = match word(i) {
                "materialized" | "live" | "window" | "row" | "settings" | "named" => {
                    format!("{} {}", word(i), word(i + 1))
                }
                other => other.to_owned(),
            };
            format!("{first} {object}").trim_end().to_owned()
        }
        "drop" | "alter" | "detach" | "show" | "exists" | "truncate" | "undrop" | "system" => {
            format!("{first} {}", word(0)).trim_end().to_owned()
        }
        _ => first,
    }
}
