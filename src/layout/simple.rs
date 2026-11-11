//! Statements of a line or two: `DROP`, `DETACH`, `TRUNCATE`, `UNDROP`,
//! `ATTACH`, `RENAME`, `EXCHANGE`, `OPTIMIZE`, `SET` and `USE`.
//!
//! A statement stays on one line if it fits. Otherwise its list (the tables
//! of `DROP`, the pairs of `RENAME`, the settings of `SET`, the columns of
//! `DEDUPLICATE BY`) goes one item per line, one level in, as a clause's
//! list does, and what follows the list starts a line of its own at the
//! statement's indentation; output clauses (`INTO OUTFILE`, `FORMAT`,
//! `SETTINGS`) start lines of their own there, as a query's do. A comment on
//! a line of its own between two parts, and what follows it, go one level
//! in. A definition kept as written after `ATTACH`'s name follows on the
//! line, re-indented as a block.

use super::shapes;
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
    // The group of the list before, if it has one, which decides whether
    // what follows the list starts a line.
    let mut list: Option<GroupId> = None;
    // An indentation for the rest of the line, opened where a comment ends
    // one between two parts.
    let mut scope = None;
    for (k, part) in parts.iter().enumerate() {
        let el_kind = part.el.kind();
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
            w.gap(part.trivia.clone(), Sep::Line, 0);
            w.el(&part.el);
        } else if el_kind.is_list() {
            list = shapes::list_body(w, part, false);
        } else {
            if let Some(group) = list.take() {
                w.gap(part.trivia.clone(), Sep::Nothing, 0);
                w.b.if_break(group, Gap::HardLine, Gap::Space);
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
    w.b.close(open);
}

#[cfg(test)]
mod tests;
