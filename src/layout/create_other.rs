//! `CREATE DATABASE` and the clauses of `CREATE DICTIONARY`; a dictionary's
//! statement is laid out as a table's (see `create`), and a function's as a
//! statement of a line or two (see `simple`).
//!
//! A database stays on one line if it fits; otherwise each clause starts a
//! line at the statement's indentation, as a table's do.
//!
//! A dictionary's `SOURCE(…)`, `LAYOUT(…)`, `LIFETIME(…)` and `RANGE(…)`
//! glue their brackets to the keyword and to the source's or layout's name,
//! as `SHOW CREATE DICTIONARY` prints them. Their key-value pairs stay on the
//! line if they fit; otherwise they go one per line, one level in, and the
//! closing brackets together start the line after them: `SOURCE(CLICKHOUSE(`,
//! the pairs, `))`.

use super::tree::El;
use super::tree::Node;
use super::tree::Part;
use super::writer::Sep;
use super::writer::Writer;
use crate::syntax::SyntaxKind;

/// The body of a `CREATE DATABASE` statement.
pub(crate) fn database(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let open = w.b.open_group();
    for (k, part) in parts.iter().enumerate() {
        if k > 0 {
            w.gap(part.trivia.clone(), Sep::Line, 0);
        }
        w.el(&part.el);
    }
    w.b.close(open);
}

/// `SOURCE(…)`, `LAYOUT(…)`, `LIFETIME(…)`, `RANGE(…)` or `SETTINGS(…)` of a
/// dictionary.
pub(crate) fn clause(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    glued_brackets(w, node.kind(), &parts);
}

/// `name(pairs)` in `SOURCE` or `LAYOUT`.
pub(crate) fn key_value_function(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    glued_brackets(w, node.kind(), &parts);
}

/// `parts`, children of a node of `parent` kind: what comes before `(` on
/// one line, `(` glued to it, and what the brackets hold: pairs as
/// [`pairs`] lays them out, anything else on the line.
fn glued_brackets(w: &mut Writer<'_>, parent: SyntaxKind, parts: &[Part]) {
    let open = parts
        .iter()
        .position(|p| p.el.is_token(SyntaxKind::OpeningRoundBracket));
    let close = parts
        .iter()
        .rposition(|p| p.el.is_token(SyntaxKind::ClosingRoundBracket));
    let (Some(open), Some(close)) = (open, close) else {
        w.inline(parent, parts);
        return;
    };
    if close < open {
        w.inline(parent, parts);
        return;
    }
    w.inline(parent, &parts[..open]);
    if open > 0 {
        w.gap(parts[open].trivia.clone(), Sep::Nothing, 0);
    }
    w.el(&parts[open].el);
    let inner = &parts[open + 1..close];
    let closer = &parts[close];
    match inner {
        [only] if only.el.kind() == SyntaxKind::KeyValuePairs => {
            let group = w.b.open_group();
            let indent = w.b.open_indent();
            w.gap(only.trivia.clone(), Sep::Soft, 0);
            if let El::Node(node) = &only.el {
                pairs(w, node);
            }
            w.gap_before_closer(closer.trivia.clone(), indent, Sep::Soft);
            w.b.close(group);
        }
        _ => {
            for (k, part) in inner.iter().enumerate() {
                let sep = if k == 0 {
                    Sep::Nothing
                } else {
                    w.join(parent, &inner[k - 1].el, &part.el)
                };
                w.gap(part.trivia.clone(), sep, 0);
                w.el(&part.el);
            }
            w.gap(closer.trivia.clone(), Sep::Nothing, 0);
        }
    }
    w.el(&closer.el);
    for (k, part) in parts.iter().enumerate().skip(close + 1) {
        let sep = w.join(parent, &parts[k - 1].el, &part.el);
        w.gap(part.trivia.clone(), sep, 0);
        w.el(&part.el);
    }
}

/// Key-value pairs, a line between each two in the group that is open.
fn pairs(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    for (k, part) in parts.iter().enumerate() {
        if k > 0 {
            w.gap(part.trivia.clone(), Sep::Line, 0);
        }
        w.el(&part.el);
    }
}

#[cfg(test)]
mod tests;
