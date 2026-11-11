//! Statements of a line or two: `DROP`, `DETACH`, `TRUNCATE`, `UNDROP`,
//! `ATTACH`, `RENAME`, `EXCHANGE`, `OPTIMIZE`, `SET` and `USE`.
//!
//! A statement stays on one line if it fits. Otherwise `ON CLUSTER`, the
//! clauses of `OPTIMIZE` after its table (`PARTITION`, `FINAL`,
//! `DEDUPLICATE BY`) and the filters of `SHOW` (`FROM`, `LIKE`, `WHERE`,
//! `LIMIT`) each start a line one level in; its list (the tables
//! of `DROP`, the pairs of `RENAME`, the settings of `SET`, the columns of
//! `DEDUPLICATE BY`) goes one item per line, one level in, as a clause's
//! list does, and what follows a list that broke starts a line of its own
//! at the statement's indentation; output clauses (`INTO OUTFILE`, `FORMAT`,
//! `SETTINGS`) start lines of their own there, as a query's do. A comment on
//! a line of its own between two parts, and what follows it, go one level
//! in. A definition kept as written after `ATTACH`'s name follows on the
//! line, re-indented as a block.

use super::shapes;
use super::tree::El;
use super::tree::Node;
use super::writer::Sep;
use super::writer::Writer;
use crate::doc::Gap;
use crate::doc::GroupId;
use crate::syntax::SyntaxKind;

/// The body of one of these statements.
pub(crate) fn statement(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    let open = w.b.open_group();
    // Everything up to the output clauses, whose parts that
    // `breaks_before` picks start lines when it breaks.
    let mut head = Some(w.b.open_group());
    // The group of the list before, if it has one, which decides whether
    // what follows the list starts a line.
    let mut list: Option<GroupId> = None;
    // An indentation for the rest of the line, opened where a comment ends
    // one between two parts.
    let mut scope = None;
    for (k, part) in parts.iter().enumerate() {
        let el_kind = part.el.kind();
        let head_break = k > 0 && breaks_before(w, kind, &part.el);
        if k == 0 {
            w.el(&part.el);
        } else if matches!(
            el_kind,
            SyntaxKind::IntoOutfileClause | SyntaxKind::FormatClause | SyntaxKind::SettingsClause
        ) {
            list = None;
            if let Some(scope) = scope.take() {
                w.b.close(scope);
            }
            if let Some(head) = head.take() {
                w.b.close(head);
            }
            w.gap(part.trivia.clone(), Sep::Line, 0);
            w.el(&part.el);
        } else if el_kind.is_list() {
            list = shapes::list_body(w, part, false);
        } else {
            if let Some(group) = list.take() {
                w.gap(part.trivia.clone(), Sep::Nothing, 0);
                // A list that broke broke the group around it, whose line
                // then starts a line at the statement's indentation.
                if head_break {
                    w.b.if_break(group, Gap::Line, Gap::Nothing);
                    if scope.is_none() {
                        scope = Some(w.b.open_indent());
                    }
                    w.b.if_break(group, Gap::Nothing, Gap::Line);
                } else {
                    w.b.if_break(group, Gap::Line, Gap::Space);
                }
            } else if head_break {
                if scope.is_none() {
                    scope = Some(w.b.open_indent());
                }
                w.gap(part.trivia.clone(), Sep::Line, 0);
            } else {
                if scope.is_none() {
                    scope = w.body_scope(part.trivia.clone());
                }
                let sep = w.join(kind, &parts[k - 1].el, &part.el);
                w.gap(part.trivia.clone(), sep, 0);
            }
            w.el(&part.el);
        }
    }
    if let Some(scope) = scope {
        w.b.close(scope);
    }
    if let Some(head) = head {
        w.b.close(head);
    }
    w.b.close(open);
}

/// Whether `el`, a part after the first of a statement of `kind`, starts a
/// line one level in when the statement breaks: `ON CLUSTER`, but in a
/// function, whose lambda breaks instead; the clauses of
/// `OPTIMIZE` after the table; and the filters of `SHOW`.
fn breaks_before(w: &Writer<'_>, kind: SyntaxKind, el: &El) -> bool {
    match (kind, el.kind()) {
        (SyntaxKind::CreateFunction, _) => false,
        (_, SyntaxKind::OnCluster)
        | (SyntaxKind::OptimizeQuery, SyntaxKind::Partition)
        | (SyntaxKind::ShowQuery, SyntaxKind::WhereClause) => true,
        (SyntaxKind::OptimizeQuery, _) => {
            shapes::keyword_is(w, el, &["FINAL", "FORCE", "DEDUPLICATE", "DRY"])
        }
        (SyntaxKind::ShowQuery, _) => shapes::keyword_is(
            w,
            el,
            &["FROM", "IN", "LIKE", "ILIKE", "NOT", "WHERE", "LIMIT"],
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
