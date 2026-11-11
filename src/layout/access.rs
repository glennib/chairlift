//! Access control: `GRANT`, `REVOKE`, and `CREATE` or `ALTER` of users,
//! roles, quotas, row policies and settings profiles.
//!
//! A statement stays on one line if it fits. Otherwise each clause starts a
//! line of its own, one level in: `ON`, `TO` or `FROM`, and `WITH … OPTION`
//! of a `GRANT` of privileges on one object, and every clause of a `CREATE`
//! or `ALTER`. The list at the head (the privileges, the roles granted, the
//! names) stays on the statement's line if it fits, and otherwise goes one
//! item per line, one level in, as a clause's list does. `ON CLUSTER` stays
//! where it is written. A `GRANT` of privileges on several objects is a list
//! of them, each with its `ON`.
//!
//! A user's host (`u@'%'`), a privilege's columns (`SELECT(a, b)`),
//! `CURRENT_USER()` and a grant's object (`db.*`) are written without
//! spaces.

use super::shapes;
use super::tree::Node;
use super::tree::Part;
use super::writer::Sep;
use super::writer::Writer;
use crate::syntax::SyntaxKind;

/// The body of a [`GrantQuery`](SyntaxKind::GrantQuery) or an
/// [`AccessEntityQuery`](SyntaxKind::AccessEntityQuery).
pub(crate) fn statement(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    let open = w.b.open_group();
    for (k, part) in parts.iter().enumerate() {
        if k == 0 {
            w.el(&part.el);
            continue;
        }
        let el_kind = part.el.kind();
        if el_kind == SyntaxKind::AccessList
            && let Some(element) = single_element(w, part)
        {
            // Privileges on one object: their `ON` is a clause of the
            // statement.
            let inner = w.parts(&element);
            for (j, piece) in inner.iter().enumerate() {
                let trivia = if j == 0 {
                    part.trivia.clone()
                } else {
                    piece.trivia.clone()
                };
                let piece = Part {
                    el: piece.el,
                    trivia,
                };
                place(w, kind, &parts[k - 1], &piece);
            }
            continue;
        }
        place(w, kind, &parts[k - 1], part);
    }
    w.b.close(open);
}

/// Adds `part`, which follows `prev` in a statement of `kind`.
fn place(w: &mut Writer<'_>, kind: SyntaxKind, prev: &Part, part: &Part) {
    let el_kind = part.el.kind();
    if breaks_before(el_kind) {
        let indent = w.b.open_indent();
        w.gap(part.trivia.clone(), Sep::Line, 0);
        w.el(&part.el);
        w.b.close(indent);
    } else if el_kind.is_list() {
        shapes::list_body(w, part, false);
    } else {
        let sep = w.join(kind, &prev.el, &part.el);
        w.gap(part.trivia.clone(), sep, 0);
        w.el(&part.el);
    }
}

/// Whether a child of this kind starts a line when its statement breaks.
fn breaks_before(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::GrantOn
            | SyntaxKind::GranteesClause
            | SyntaxKind::GrantOptionClause
            | SyntaxKind::AccessClause
            | SyntaxKind::VerbatimClause
    )
}

/// The one [`AccessElement`](SyntaxKind::AccessElement) of the
/// [`AccessList`](SyntaxKind::AccessList) `part`, if it has one and no
/// comma.
fn single_element<'g>(w: &Writer<'_>, part: &Part<'g>) -> Option<Node<'g>> {
    let list = part.el.node()?;
    let items = w.parts(&list);
    match items.as_slice() {
        [only] if only.el.kind() == SyntaxKind::AccessElement => only.el.node(),
        _ => None,
    }
}

/// A node whose children touch: a [`UserName`](SyntaxKind::UserName), a
/// [`RoleKeyword`](SyntaxKind::RoleKeyword), a
/// [`GrantTarget`](SyntaxKind::GrantTarget).
pub(crate) fn glued(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    for (k, part) in parts.iter().enumerate() {
        if k > 0 {
            w.gap(part.trivia.clone(), Sep::Nothing, 0);
        }
        w.el(&part.el);
    }
}

/// A [`Privilege`](SyntaxKind::Privilege) or
/// [`CurrentGrants`](SyntaxKind::CurrentGrants): its words spaced, the
/// bracket after them touching the last.
pub(crate) fn privilege(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    for (k, part) in parts.iter().enumerate() {
        if k > 0 {
            let sep = if part.el.kind() == SyntaxKind::ParenList
                || part.el.is_token(SyntaxKind::OpeningRoundBracket)
            {
                Sep::Nothing
            } else {
                w.join(node.kind(), &parts[k - 1].el, &part.el)
            };
            w.gap(part.trivia.clone(), sep, 0);
        }
        w.el(&part.el);
    }
}

#[cfg(test)]
mod tests;
