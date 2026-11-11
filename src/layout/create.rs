//! `CREATE TABLE` and `CREATE MATERIALIZED VIEW`: the header line, the
//! element list with `(` alone on its line and one element per line, and
//! the clauses one per line in the order written.
//!
//! The header line holds the statement's keywords and name and the clauses
//! that read as part of it, when they follow the name directly: a view's
//! `REFRESH` and `TO`, and `EMPTY`, `CLONE`, `POPULATE` and `AS` a table or
//! table function. They stay on the line if it fits; otherwise each goes on
//! a line of its own, one level in.
//!
//! `AS` and a query end the line of the header, of the element list's `)`,
//! or of `EMPTY`, `CLONE` or `POPULATE`: `… TO t AS`. After a storage clause
//! `AS` goes on a line of its own, where it cannot read as an alias of what
//! ends the line before. The query starts the next line at the statement's
//! indentation, so that a view's clauses line up as a plain query's do; a
//! query alone in brackets hugs them, as a subquery does.

use super::query;
use super::shapes;
use super::tree::El;
use super::tree::Node;
use super::tree::Part;
use super::writer::Sep;
use super::writer::Writer;
use crate::syntax::SyntaxKind;

/// The body of a `CREATE TABLE` or `CREATE MATERIALIZED VIEW` statement.
pub(crate) fn statement(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let Some((first, rest)) = parts.split_first() else {
        return;
    };
    let header = w.b.open_group();
    // What follows a comment inside the header continues it, one level in.
    let indent = w.b.open_indent();
    w.el(&first.el);
    let on_line = rest
        .iter()
        .take_while(|part| on_header_line(w, part))
        .count();
    for part in &rest[..on_line] {
        w.gap(part.trivia.clone(), Sep::Line, 0);
        w.el(&part.el);
    }
    w.b.close(indent);
    w.b.close(header);
    for (k, part) in rest.iter().enumerate().skip(on_line) {
        let (sep, blank) = match part.el.kind() {
            SyntaxKind::TableElementList => (Sep::Hard, 0),
            // The header group counts `AS` in its measure, as it follows
            // the group on its line.
            SyntaxKind::AsClause if holds_query(w, part) && ends_line_with_as(&rest[..k]) => {
                (Sep::Space, 0)
            }
            _ => (Sep::Hard, 1),
        };
        w.gap(part.trivia.clone(), sep, blank);
        w.el(&part.el);
    }
}

/// Whether `AS` and a query go at the end of the line of what is before
/// them, `before` the parts after the header: the header line, the element
/// list, or `EMPTY`, `CLONE` or `POPULATE`.
fn ends_line_with_as(before: &[Part]) -> bool {
    before.last().is_none_or(|part| {
        matches!(
            part.el.kind(),
            SyntaxKind::ToClause
                | SyntaxKind::RefreshClause
                | SyntaxKind::PopulateClause
                | SyntaxKind::AsClause
                | SyntaxKind::TableElementList
        )
    })
}

/// Whether `part`, a child of a `CREATE` statement after its header, stays
/// on the header line when it follows the header directly.
fn on_header_line(w: &Writer<'_>, part: &Part) -> bool {
    match part.el.kind() {
        SyntaxKind::ToClause | SyntaxKind::RefreshClause | SyntaxKind::PopulateClause => true,
        SyntaxKind::AsClause => !holds_query(w, part),
        _ => false,
    }
}

/// Whether `part` is an [`AsClause`](SyntaxKind::AsClause) holding a query.
fn holds_query(w: &Writer<'_>, part: &Part) -> bool {
    part.el
        .node()
        .is_some_and(|node| query_at(w, &node).is_some())
}

/// Where the query of `node`, an [`AsClause`](SyntaxKind::AsClause), is
/// among its parts, if it holds one rather than a table or a table
/// function.
fn query_at(w: &Writer<'_>, node: &Node) -> Option<usize> {
    if node.kind() != SyntaxKind::AsClause {
        return None;
    }
    let parts = w.parts(node);
    let body = parts.iter().position(|p| matches!(p.el, El::Node(_)))?;
    matches!(
        parts[body].el.kind(),
        SyntaxKind::SelectQuery | SyntaxKind::UnionQuery
    )
    .then_some(body)
}

/// `AS` and a table or a table function, on one line; or `AS` and a query,
/// which starts the next line at the indentation of the line `AS` is on,
/// or hugs its brackets after `AS (` if it is alone in them.
pub(crate) fn as_clause(w: &mut Writer<'_>, node: &Node) {
    let Some(body) = query_at(w, node) else {
        w.generic(node);
        return;
    };
    let parts = w.parts(node);
    w.inline(node.kind(), &parts[..body]);
    for (k, part) in parts[body..].iter().enumerate() {
        // A query alone in brackets hugs them: `AS (` ends the line.
        let sep = if k == 0 && in_brackets(w, &part.el) {
            Sep::Space
        } else {
            Sep::Hard
        };
        w.gap(part.trivia.clone(), sep, 0);
        match &part.el {
            El::Node(node) => query::query(w, node),
            el @ El::Token { .. } => w.el(el),
        }
    }
}

/// Whether `el` is a query alone in brackets: a union of one bracketed
/// query.
fn in_brackets(w: &Writer<'_>, el: &El) -> bool {
    el.node().is_some_and(|node| {
        node.kind() == SyntaxKind::UnionQuery
            && matches!(w.parts(&node).as_slice(), [only] if only.el.kind() == SyntaxKind::Subquery)
    })
}

/// `( elements )` of a table, a view or a `TimeSeries` target: `(` where
/// the caller puts it (alone on its line), each element on a line of its
/// own, one level in, and `)` at the indentation of `(`.
pub(crate) fn element_list(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let (Some(open), Some(close)) = (parts.first(), parts.last()) else {
        return;
    };
    if parts.len() < 2
        || !open.el.is_token(SyntaxKind::OpeningRoundBracket)
        || !close.el.is_token(SyntaxKind::ClosingRoundBracket)
    {
        w.generic(node);
        return;
    }
    w.el(&open.el);
    let inner = &parts[1..parts.len() - 1];
    if inner.is_empty() {
        w.gap(close.trivia.clone(), Sep::Nothing, 0);
    } else {
        let list = shapes::split_items(inner);
        let group = w.b.open_group();
        w.b.force_break();
        let indent = w.b.open_indent();
        shapes::items(w, node.kind(), &list, Sep::Line, None);
        w.gap_before_closer(close.trivia.clone(), indent, Sep::Line);
        w.b.close(group);
    }
    w.el(&close.el);
}

/// A column definition: its name, type and modifiers on one line if they
/// fit; otherwise each modifier but `NULL` and `COLLATE`, which belong to
/// the type, on a line of its own, one level in.
pub(crate) fn column(w: &mut Writer<'_>, node: &Node) {
    breaking(w, node, |_, el| {
        matches!(
            el.kind(),
            SyntaxKind::ColumnDefault
                | SyntaxKind::ColumnCodec
                | SyntaxKind::ColumnStatistics
                | SyntaxKind::ColumnTtl
                | SyntaxKind::ColumnComment
                | SyntaxKind::ColumnPrimaryKey
                | SyntaxKind::ColumnSettings
        )
    });
}

/// `INDEX name expr TYPE type GRANULARITY n`: on one line if it fits;
/// otherwise a line break, one level in, before `TYPE` and `GRANULARITY`.
pub(crate) fn index(w: &mut Writer<'_>, node: &Node) {
    breaking(w, node, |w, el| keyword_is(w, el, &["TYPE", "GRANULARITY"]));
}

/// `PROJECTION name (query)`, the query hugging its brackets; or
/// `PROJECTION name INDEX exprs TYPE type` as an index is laid out, with a
/// line break before `WITH SETTINGS` too.
pub(crate) fn projection(w: &mut Writer<'_>, node: &Node) {
    if w.parts(node)
        .iter()
        .any(|p| p.el.kind() == SyntaxKind::Subquery)
    {
        w.generic(node);
    } else {
        breaking(w, node, |w, el| {
            el.kind() == SyntaxKind::ProjectionSettings || keyword_is(w, el, &["TYPE"])
        });
    }
}

/// `REFRESH EVERY interval …`: on one line if it fits; otherwise a line
/// break, one level in, before `OFFSET`, `RANDOMIZE FOR`, `DEPENDS ON`,
/// `SETTINGS` and `APPEND`.
pub(crate) fn refresh(w: &mut Writer<'_>, node: &Node) {
    breaking(w, node, |w, el| {
        matches!(
            el.kind(),
            SyntaxKind::DependsOn | SyntaxKind::SettingsClause
        ) || keyword_is(w, el, &["OFFSET", "RANDOMIZE", "APPEND"])
    });
}

/// One rule of a table's `TTL`: on one line if it fits; otherwise a line
/// break, one level in, before `WHERE`, `GROUP BY` and `SET`. The rule's
/// expression breaks as it would on its own, at the indentation of the
/// line it starts on.
pub(crate) fn ttl_rule(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    let Some((first, rest)) = parts.split_first() else {
        return;
    };
    w.frozen_before(w.range(node).0);
    let group = w.b.open_group();
    w.el(&first.el);
    let indent = w.b.open_indent();
    for (k, part) in rest.iter().enumerate() {
        let sep = if keyword_is(w, &part.el, &["WHERE", "GROUP", "SET"]) {
            Sep::Line
        } else {
            w.join(kind, &parts[k].el, &part.el)
        };
        w.gap(part.trivia.clone(), sep, 0);
        w.el(&part.el);
    }
    w.b.close(indent);
    w.b.close(group);
}

/// Whether `el` is a keyword token whose text is one of `words`.
fn keyword_is(w: &Writer<'_>, el: &El, words: &[&str]) -> bool {
    match el {
        El::Token {
            kind: SyntaxKind::Keyword,
            index,
        } => {
            let text = w.text(*index);
            words.iter().any(|word| word.eq_ignore_ascii_case(text))
        }
        _ => false,
    }
}

/// Adds `node`'s children on one line if they fit, spaced as
/// [`Writer::join`] says; otherwise a line break, one level in, before each
/// child after the first that `breaks` picks.
fn breaking(w: &mut Writer<'_>, node: &Node, breaks: impl Fn(&Writer<'_>, &El) -> bool) {
    let parts = w.parts(node);
    let kind = node.kind();
    if parts.is_empty() {
        return;
    }
    w.frozen_before(w.range(node).0);
    let group = w.b.open_group();
    let indent = w.b.open_indent();
    w.inline_with(kind, &parts, |w, _, next| {
        breaks(w, next).then_some(Sep::Line)
    });
    w.b.close(indent);
    w.b.close(group);
}

/// `CODEC(…)` and `STATISTICS(…)`: the keyword and its list without a
/// space between them.
pub(crate) fn codec(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    w.inline_with(node.kind(), &parts, |_, _, next| {
        (next.kind() == SyntaxKind::CodecList).then_some(Sep::Nothing)
    });
}

/// `DEFINER = user@host SQL SECURITY …`: on one line, with no space around
/// the `@` of a user name.
pub(crate) fn sql_security(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    w.inline_with(node.kind(), &parts, |_, prev, next| {
        (prev.is_token(SyntaxKind::At) || next.is_token(SyntaxKind::At)).then_some(Sep::Nothing)
    });
}

/// Whether `node`, an [`OrderByClause`](SyntaxKind::OrderByClause), is a
/// table's: one key expression, not a list of items.
pub(crate) fn is_table_order_by(w: &Writer<'_>, node: &Node) -> bool {
    !w.parts(node)
        .iter()
        .any(|p| p.el.kind() == SyntaxKind::OrderByList)
}

/// `(a DESC, b)` or `tuple(a DESC, b)` of a table's `ORDER BY`: a list in
/// brackets that stays on its line if it fits, otherwise one key per line.
pub(crate) fn order_by_tuple(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let Some(open) = parts
        .iter()
        .position(|p| p.el.is_token(SyntaxKind::OpeningRoundBracket))
    else {
        w.generic(node);
        return;
    };
    // The keys are the items of the list node between the brackets; the
    // list's first item takes the trivia before the list.
    let mut flat: Vec<Part> = Vec::with_capacity(parts.len() * 2);
    for part in &parts[open..] {
        match part.el.node() {
            Some(list) if list.kind() == SyntaxKind::OrderByList => {
                let mut items = w.parts(&list);
                if let Some(first) = items.first_mut() {
                    first.trivia = part.trivia.clone();
                }
                flat.extend(items);
            }
            _ => flat.push(part.clone()),
        }
    }
    w.inline(node.kind(), &parts[..open]);
    if open > 0 {
        w.gap(parts[open].trivia.clone(), Sep::Nothing, 0);
    }
    shapes::bracketed_parts(w, SyntaxKind::OrderByList, &flat);
}

/// A target of a `TimeSeries` table: its keywords and what follows them on
/// one line; its column list as a table's, `(` alone on its line; its
/// storage clauses each on a line of their own, one level in.
pub(crate) fn view_target(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    let mut indent = None;
    for (k, part) in parts.iter().enumerate() {
        let el_kind = part.el.kind();
        if el_kind == SyntaxKind::TableElementList {
            w.gap(part.trivia.clone(), Sep::Hard, 0);
        } else if el_kind.is_clause() {
            if indent.is_none() {
                indent = Some(w.b.open_indent());
            }
            w.gap(part.trivia.clone(), Sep::Hard, 1);
        } else if k > 0 {
            let sep = w.join(kind, &parts[k - 1].el, &part.el);
            w.gap(part.trivia.clone(), sep, 0);
        }
        w.el(&part.el);
    }
    if let Some(indent) = indent {
        w.b.close(indent);
    }
}

#[cfg(test)]
mod tests;
