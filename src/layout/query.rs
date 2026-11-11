//! `SELECT` queries: clauses one per line, set operations, subqueries and
//! CTEs.
//!
//! [`query`] lays out a query at the indentation of the scope it is called
//! in, so a statement that holds one (a materialized view's `AS SELECT`,
//! `CREATE TABLE … AS SELECT`, `INSERT … SELECT`) puts its own header on a
//! line, breaks the line and calls it at column 0.

use super::expr;
use super::shapes;
use super::tree::El;
use super::tree::Node;
use super::writer::Sep;
use super::writer::Writer;
use crate::doc::Gap;
use crate::lexer::TokenKind;
use crate::syntax::SyntaxKind;

/// A query: a [`SelectQuery`](SyntaxKind::SelectQuery) or a
/// [`UnionQuery`](SyntaxKind::UnionQuery), or any node for which the
/// generic rules do.
pub(crate) fn query(w: &mut Writer<'_>, node: &Node) {
    match node.kind() {
        SyntaxKind::SelectQuery | SyntaxKind::UnionQuery => clauses(w, node),
        _ => super::node(w, node),
    }
}

/// A query's children, one per line: clauses, and the operands and
/// operators of a set operation, which stay flat at one indentation
/// whatever their priorities. At most one blank line is kept before each.
pub(crate) fn clauses(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    for (k, part) in parts.iter().enumerate() {
        if k > 0 {
            // A comma join continues the line of the join before it.
            let sep = if shapes::starts_with(w, &part.el, TokenKind::Comma) {
                Sep::Nothing
            } else {
                Sep::Hard
            };
            w.gap(part.trivia.clone(), sep, 1);
        }
        w.el(&part.el);
    }
}

/// `SELECT [DISTINCT] [TOP n] items`: one item on the `SELECT` line if it
/// fits, otherwise on the next, indented; more than one item one per line,
/// always.
pub(crate) fn select_clause(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    let list = parts
        .iter()
        .rposition(|p| p.el.kind() == SyntaxKind::SelectList);
    let head = list.unwrap_or(parts.len());
    let list_node = list.and_then(|i| parts[i].el.node());
    let list_parts = list_node.as_ref().map(|n| w.parts(n)).unwrap_or_default();
    let items = shapes::split_items(&list_parts);
    let open = w.b.open_group();
    if shapes::count_items(&items) > 1 {
        w.b.force_break();
    }
    w.inline(kind, &parts[..head]);
    if let Some(i) = list {
        let indent = w.b.open_indent();
        shapes::items(
            w,
            SyntaxKind::SelectList,
            &items,
            Sep::Line,
            Some(parts[i].trivia.clone()),
        );
        w.b.close(indent);
        for part in &parts[i + 1..] {
            w.gap(part.trivia.clone(), Sep::Space, 0);
            w.el(&part.el);
        }
    }
    w.b.close(open);
}

/// A clause of keywords and a list: `FROM`, `ARRAY JOIN`, `ORDER BY`,
/// `SETTINGS`, `WINDOW`: the list on the keywords' line if it fits,
/// otherwise one item per line, indented.
pub(crate) fn list_clause(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    let n = shapes::keywords(w, kind, &parts);
    let mut prev: Option<&El> = n.checked_sub(1).map(|i| &parts[i].el);
    for part in &parts[n..] {
        if part.el.node().is_some_and(|n| n.kind().is_list()) {
            shapes::list_body(w, part, false);
        } else {
            let sep = prev.map_or(Sep::Nothing, |prev| w.join(kind, prev, &part.el));
            w.gap(part.trivia.clone(), sep, 0);
            w.el(&part.el);
        }
        prev = Some(&part.el);
    }
}

/// `GROUP BY keys [WITH TOTALS]`: the keys as a list; a modifier on their
/// line if they fit on one, otherwise on a line of its own after them.
pub(crate) fn group_by(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    let n = shapes::keywords(w, kind, &parts);
    let mut keys = None;
    let mut prev: Option<&El> = n.checked_sub(1).map(|i| &parts[i].el);
    for part in &parts[n..] {
        match part.el.kind() {
            SyntaxKind::ExprList => keys = shapes::list_body(w, part, false),
            SyntaxKind::GroupByModifier => {
                if prev.is_none() {
                    w.gap(part.trivia.clone(), Sep::Nothing, 0);
                } else if let Some(keys) = keys {
                    w.gap(part.trivia.clone(), Sep::Nothing, 0);
                    w.b.if_break(keys, Gap::HardLine, Gap::Space);
                } else {
                    w.gap(part.trivia.clone(), Sep::Space, 0);
                }
                w.el(&part.el);
            }
            _ => {
                let sep = prev.map_or(Sep::Nothing, |prev| w.join(kind, prev, &part.el));
                w.gap(part.trivia.clone(), sep, 0);
                w.el(&part.el);
            }
        }
        prev = Some(&part.el);
    }
}

/// `WITH`: a single CTE after `WITH`, hugging its query; anything else a
/// list under `WITH`, one item per line unless they fit on its line.
pub(crate) fn with_clause(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    let n = shapes::keywords(w, kind, &parts);
    let Some(list_part) = parts.get(n) else {
        return;
    };
    let Some(list) = list_part
        .el
        .node()
        .filter(|l| l.kind() == SyntaxKind::WithList)
    else {
        w.inline(kind, &parts[n.saturating_sub(1)..]);
        return;
    };
    let list_parts = w.parts(&list);
    let items = shapes::split_items(&list_parts);
    let single_cte = items.len() == 1
        && items[0].comma.is_none()
        && items[0].parts.len() == 1
        && items[0].parts[0].el.kind() == SyntaxKind::CommonTableExpr;
    if single_cte {
        shapes::items(
            w,
            SyntaxKind::WithList,
            &items,
            Sep::Space,
            Some(list_part.trivia.clone()),
        );
    } else {
        let open = w.b.open_group();
        let indent = w.b.open_indent();
        shapes::items(
            w,
            SyntaxKind::WithList,
            &items,
            Sep::Line,
            Some(list_part.trivia.clone()),
        );
        w.b.close(indent);
        w.b.close(open);
    }
    for part in &parts[n + 1..] {
        w.gap(part.trivia.clone(), Sep::Space, 0);
        w.el(&part.el);
    }
}

/// A join on a line of its own: its keywords and table, then its condition
/// on the same line if the whole join fits, otherwise `ON` on the next
/// line, one level in, with each `AND` of the condition on a line of its
/// own at the same indentation. A comma join after a join continues its
/// line, and its table starts the next.
pub(crate) fn join(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    if parts
        .first()
        .is_some_and(|p| p.el.is_token(SyntaxKind::Comma))
    {
        w.el(&parts[0].el);
        for (k, part) in parts.iter().enumerate().skip(1) {
            let sep = if k == 1 {
                Sep::Hard
            } else {
                w.join(kind, &parts[k - 1].el, &part.el)
            };
            w.gap(part.trivia.clone(), sep, 0);
            w.el(&part.el);
        }
        return;
    }
    let condition = parts
        .iter()
        .position(|p| matches!(p.el.kind(), SyntaxKind::JoinOn | SyntaxKind::JoinUsing));
    let head = condition.unwrap_or(parts.len());
    let open = w.b.open_group();
    w.inline(kind, &parts[..head]);
    if let Some(i) = condition {
        let indent = w.b.open_indent();
        for part in &parts[i..] {
            w.gap(part.trivia.clone(), Sep::Line, 0);
            match &part.el {
                El::Node(n) if n.kind() == SyntaxKind::JoinOn => join_on(w, n),
                el => w.el(el),
            }
        }
        w.b.close(indent);
    }
    w.b.close(open);
}

/// `ON` and a join's condition, its `AND`s and `OR`s breaking in the
/// group of the join, at the indentation of `ON`.
fn join_on(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    for (k, part) in parts.iter().enumerate() {
        if k > 0 {
            let sep = w.join(kind, &parts[k - 1].el, &part.el);
            w.gap(part.trivia.clone(), sep, 0);
        }
        match &part.el {
            El::Node(n) if expr::is_condition_run(n) => expr::conditions(w, n, false),
            el => w.el(el),
        }
    }
}

/// `( tables and joins )` standing for one table: the tables and each
/// join on lines of their own inside the hugging brackets.
pub(crate) fn paren_join(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let (Some(first), Some(last)) = (parts.first(), parts.last()) else {
        return;
    };
    if parts.len() < 3
        || !first.el.is_token(SyntaxKind::OpeningRoundBracket)
        || !last.el.is_token(SyntaxKind::ClosingRoundBracket)
    {
        w.generic(node);
        return;
    }
    w.el(&first.el);
    let hug = w.b.open_hug();
    let indent = w.b.open_indent();
    for part in &parts[1..parts.len() - 1] {
        let sep = if shapes::starts_with(w, &part.el, TokenKind::Comma) {
            Sep::Nothing
        } else {
            Sep::Hard
        };
        w.gap(part.trivia.clone(), sep, 0);
        if part.el.kind() == SyntaxKind::TableList {
            if let Some(list) = part.el.node() {
                shapes::plain_list(w, &list);
            }
        } else {
            w.el(&part.el);
        }
    }
    w.b.close(indent);
    w.gap(last.trivia.clone(), Sep::Hard, 0);
    w.el(&last.el);
    w.b.close(hug);
}
