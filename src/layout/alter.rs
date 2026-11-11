//! `ALTER` and the lightweight `DELETE` and `UPDATE`.
//!
//! An `ALTER`'s head is on one line if it fits; otherwise `ON CLUSTER`
//! starts a line one level in. Its commands are a list. One command stays
//! on the `ALTER` line if it fits, and goes on the next line, one level in,
//! if it does not or the head broke;
//! several go one per line, one level in, each but the last with its comma.
//! Output clauses after the commands (`SETTINGS mutations_sync = 2`) stay on
//! the line if the statement fits, and start a line at the statement's
//! indentation otherwise.
//!
//! A command is on one line if it fits; otherwise `IN PARTITION` and
//! `WHERE` start lines of their own, one level in, and its definitions,
//! lists and expressions break as their own rules say. The query of
//! `MODIFY QUERY` starts the next line at the indentation of the command, or
//! at the statement's when it is the only command, as the query after
//! `CREATE … AS` does.
//!
//! `DELETE` and `UPDATE` are laid out as a query is: the head (`DELETE FROM
//! t`, `UPDATE t`) on its line, with `ON CLUSTER` on the next, one level
//! in, if it does not fit; then `SET`, `IN PARTITION`, `WHERE` and
//! `SETTINGS` each on a line of their own.

use super::query;
use super::shapes;
use super::tree::El;
use super::tree::Node;
use super::tree::Part;
use super::writer::Sep;
use super::writer::Writer;
use crate::doc::Gap;
use crate::doc::GroupId;
use crate::syntax::SyntaxKind;

/// The body of an `ALTER` statement, an
/// [`AlterQuery`](SyntaxKind::AlterQuery).
pub(crate) fn statement(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let Some(list) = parts
        .iter()
        .position(|p| p.el.kind() == SyntaxKind::AlterCommandList)
    else {
        w.generic(node);
        return;
    };
    let open = w.b.open_group();
    let head_group = shapes::head(w, node.kind(), &parts[..list], |_, el| {
        el.kind() == SyntaxKind::OnCluster
    });
    let cluster = parts[..list]
        .iter()
        .any(|p| p.el.kind() == SyntaxKind::OnCluster);
    commands(w, &parts[list], head_group.filter(|_| cluster));
    for part in &parts[list + 1..] {
        w.gap(part.trivia.clone(), Sep::Line, 0);
        w.el(&part.el);
    }
    w.b.close(open);
}

/// The command list, `part` of the statement. One command stays on the
/// line before it unless `head`, the group of a head that can break, broke.
fn commands(w: &mut Writer<'_>, part: &Part, head: Option<GroupId>) {
    let Some(list) = part.el.node() else {
        w.gap(part.trivia.clone(), Sep::Space, 0);
        w.el(&part.el);
        return;
    };
    let parts = w.parts(&list);
    let items = shapes::split_items(&parts);
    let count = shapes::count_items(&items);
    let lead = Some(part.trivia.clone());
    if count == 1 && items.len() == 1 && items[0].parts.iter().any(|p| holds_query(w, &p.el)) {
        shapes::items(w, list.kind(), &items, Sep::Space, lead);
        return;
    }
    if let Some(head) = head.filter(|_| count == 1) {
        // A head that broke broke the statement's group too, whose line
        // then starts the command's line.
        let indent = w.b.open_indent();
        w.gap(part.trivia.clone(), Sep::Nothing, 0);
        w.b.if_break(head, Gap::Line, Gap::Nothing);
        let group = w.b.open_group();
        w.b.if_break(head, Gap::Nothing, Gap::Line);
        shapes::items(w, list.kind(), &items, Sep::Nothing, None);
        w.b.close(group);
        w.b.close(indent);
        return;
    }
    let group = w.b.open_group();
    if count > 1 {
        w.b.force_break();
    }
    let indent = w.b.open_indent();
    shapes::items(w, list.kind(), &items, Sep::Line, lead);
    w.b.close(indent);
    w.b.close(group);
}

/// Whether `el` is a command holding a query, `MODIFY QUERY SELECT …`.
fn holds_query(w: &Writer<'_>, el: &El) -> bool {
    el.node().is_some_and(|node| {
        node.kind() == SyntaxKind::AlterCommand && w.parts(&node).iter().any(|p| is_query(&p.el))
    })
}

/// Whether `el` is a query.
fn is_query(el: &El) -> bool {
    matches!(el.kind(), SyntaxKind::SelectQuery | SyntaxKind::UnionQuery)
}

/// One command of `ALTER`, an [`AlterCommand`](SyntaxKind::AlterCommand).
pub(crate) fn command(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    w.frozen_before(w.range(node).0);
    let group = w.b.open_group();
    // Opened at the first part that starts a line one level in, so that a
    // list before it indents from the command's line only once.
    let mut indent = None;
    for (k, part) in parts.iter().enumerate() {
        match &part.el {
            El::Node(body) if is_query(&part.el) => {
                // The query starts a line at the indentation of the
                // command's first line.
                if let Some(indent) = indent.take() {
                    w.b.close(indent);
                }
                w.gap(part.trivia.clone(), Sep::Hard, 0);
                query::query(w, body);
            }
            el => {
                if k > 0 {
                    let sep =
                        if matches!(el.kind(), SyntaxKind::WhereClause | SyntaxKind::InPartition) {
                            if indent.is_none() {
                                indent = Some(w.b.open_indent());
                            }
                            Sep::Line
                        } else {
                            w.join(kind, &parts[k - 1].el, el)
                        };
                    w.gap(part.trivia.clone(), sep, 0);
                }
                w.el(el);
            }
        }
    }
    if let Some(indent) = indent {
        w.b.close(indent);
    }
    w.b.close(group);
}

/// The body of a `DELETE` or `UPDATE` statement: the head on one line, then
/// each clause on a line of its own.
pub(crate) fn mutation(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let head = parts
        .iter()
        .position(|p| {
            matches!(
                p.el.kind(),
                SyntaxKind::UpdateSetClause
                    | SyntaxKind::InPartition
                    | SyntaxKind::WhereClause
                    | SyntaxKind::SettingsClause
            )
        })
        .unwrap_or(parts.len());
    shapes::head(w, node.kind(), &parts[..head], |_, el| {
        el.kind() == SyntaxKind::OnCluster
    });
    for part in &parts[head..] {
        w.gap(part.trivia.clone(), Sep::Hard, 1);
        w.el(&part.el);
    }
}

#[cfg(test)]
mod tests;
