//! Shapes that many rules share: lists, clauses with a keyword line and an
//! indented body, and brackets that hug what they hold.

use super::tree::El;
use super::tree::Node;
use super::tree::Part;
use super::writer::CloserLine;
use super::writer::Sep;
use super::writer::Writer;
use crate::doc::GroupId;
use crate::lexer::TokenKind;
use crate::syntax::SyntaxKind;

/// One item of a list: its children, and the comma after it.
pub(crate) struct Item<'p, 'g> {
    pub(crate) parts: &'p [Part<'g>],
    pub(crate) comma: Option<&'p Part<'g>>,
}

/// Splits `parts` at their commas into items. A trailing comma ends the
/// last item; nothing between two commas is an item with no parts.
pub(crate) fn split_items<'p, 'g>(parts: &'p [Part<'g>]) -> Vec<Item<'p, 'g>> {
    let mut items = Vec::new();
    let mut start = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.el.is_token(SyntaxKind::Comma) {
            items.push(Item {
                parts: &parts[start..i],
                comma: Some(part),
            });
            start = i + 1;
        }
    }
    if start < parts.len() {
        items.push(Item {
            parts: &parts[start..],
            comma: None,
        });
    }
    items
}

/// The number of items with something in them.
pub(crate) fn count_items(items: &[Item<'_, '_>]) -> usize {
    items.iter().filter(|item| !item.parts.is_empty()).count()
}

/// Adds the items of a list of `parent` kind: the first after `first`
/// (which goes before the first item's trivia, or before `lead`, the trivia
/// that comes before the list itself, when given), each further one after a
/// line in the current group. A comma follows its item; a comment before a
/// comma pins it to the start of the next line, and the item after it
/// follows on that line. Blank lines are kept between items, one at most.
pub(crate) fn items(
    w: &mut Writer<'_>,
    parent: SyntaxKind,
    items: &[Item<'_, '_>],
    first: Sep,
    lead: Option<std::ops::Range<usize>>,
) {
    let mut pinned = false;
    let mut lead = lead;
    for (k, item) in items.iter().enumerate() {
        let sep = if k == 0 {
            first
        } else if pinned {
            Sep::Space
        } else {
            Sep::Line
        };
        let blank = u8::from(k > 0);
        if let Some((head, rest)) = item.parts.split_first() {
            // A list never starts with trivia, so before its first item
            // there is only what comes before the list.
            let trivia = lead.take().unwrap_or_else(|| head.trivia.clone());
            w.gap(trivia, sep, blank);
            w.el(&head.el);
            let parts = item.parts;
            for (j, part) in rest.iter().enumerate() {
                let sep = w.join(parent, &parts[j].el, &part.el);
                w.gap(part.trivia.clone(), sep, 0);
                w.el(&part.el);
            }
        } else if k == 0
            && let Some(lead) = lead.take()
        {
            w.gap(lead, sep, 0);
        }
        pinned = false;
        if let Some(comma) = item.comma {
            pinned = w.gap(comma.trivia.clone(), Sep::Nothing, 0);
            w.el(&comma.el);
        }
    }
}

/// Whether `kind` opens a bracket.
fn opens(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::OpeningRoundBracket
            | SyntaxKind::OpeningSquareBracket
            | SyntaxKind::OpeningCurlyBrace
    )
}

/// Whether `kind` closes a bracket.
fn closes(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::ClosingRoundBracket
            | SyntaxKind::ClosingSquareBracket
            | SyntaxKind::ClosingCurlyBrace
    )
}

/// Adds a bracketed list, `( a, b )` with anything before the opening
/// bracket (`ARRAY`) on one line: the items on the bracket's line if they
/// fit, otherwise one per line, indented, and the closing bracket at the
/// indentation of the line the list starts on.
pub(crate) fn bracketed(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    bracketed_parts(w, node.kind(), &parts);
}

/// [`bracketed`] for the children `parts` of a node of `parent` kind.
pub(crate) fn bracketed_parts(w: &mut Writer<'_>, parent: SyntaxKind, parts: &[Part]) {
    let open = parts.iter().position(|p| opens(p.el.kind()));
    let close = parts.iter().rposition(|p| closes(p.el.kind()));
    let (Some(open), Some(close)) = (open, close) else {
        w.inline(parent, parts);
        return;
    };
    if close < open {
        w.inline(parent, parts);
        return;
    }
    w.inline(parent, &parts[..=open]);
    let inner = &parts[open + 1..close];
    let closer = &parts[close];
    let mut group = None;
    if inner.is_empty() {
        w.gap(closer.trivia.clone(), Sep::Nothing, 0);
    } else {
        let list = split_items(inner);
        let hug = w.b.open_hug();
        let open = w.b.open_group();
        group = open.group();
        let indent = w.b.open_indent();
        items(w, parent, &list, Sep::Soft, None);
        w.b.close(indent);
        w.gap(closer.trivia.clone(), Sep::Soft, 0);
        w.b.close(open);
        w.b.close(hug);
    }
    w.el(&closer.el);
    if let (Some(group), El::Token { index, .. }) = (group, &closer.el) {
        w.closed(*index, CloserLine::IfBroken(group));
    }
    for (k, part) in parts.iter().enumerate().skip(close + 1) {
        let sep = w.join(parent, &parts[k - 1].el, &part.el);
        w.gap(part.trivia.clone(), sep, 0);
        w.el(&part.el);
    }
}

/// Adds a list without brackets: the first item where the list starts, each
/// further one on a line of its own, indented, unless they all fit.
pub(crate) fn plain_list(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let list = split_items(&parts);
    let open = w.b.open_group();
    let indent = w.b.open_indent();
    items(w, node.kind(), &list, Sep::Nothing, None);
    w.b.close(indent);
    w.b.close(open);
}

/// Adds a clause's leading keywords, from `parts`, spaced; returns how many
/// parts they are.
pub(crate) fn keywords(w: &mut Writer<'_>, parent: SyntaxKind, parts: &[Part]) -> usize {
    let n = parts
        .iter()
        .take_while(|p| matches!(p.el, El::Token { .. }))
        .count();
    w.inline(parent, &parts[..n]);
    n
}

/// Adds a list after a clause's keywords, which are already added: on the
/// keywords' line if it fits, otherwise one item per line, indented, under
/// them; a single item after a space, breaking inside as its own rules say.
/// `list_part` is the list's part among the clause's children. Returns the
/// group the items are in, if they are in one.
pub(crate) fn list_body(w: &mut Writer<'_>, list_part: &Part, force: bool) -> Option<GroupId> {
    let Some(list) = list_part.el.node() else {
        w.gap(list_part.trivia.clone(), Sep::Space, 0);
        w.el(&list_part.el);
        return None;
    };
    if list.kind().is_verbatim() {
        w.gap(list_part.trivia.clone(), Sep::Space, 0);
        w.el(&list_part.el);
        return None;
    }
    let parts = w.parts(&list);
    let all = split_items(&parts);
    if count_items(&all) <= 1 && !force && all.iter().all(|i| i.comma.is_none()) {
        let scope = w.body_scope(list_part.trivia.clone());
        items(
            w,
            list.kind(),
            &all,
            Sep::Space,
            Some(list_part.trivia.clone()),
        );
        if let Some(scope) = scope {
            w.b.close(scope);
        }
        return None;
    }
    let open = w.b.open_group();
    if force {
        w.b.force_break();
    }
    let id = open.group();
    let indent = w.b.open_indent();
    items(
        w,
        list.kind(),
        &all,
        Sep::Line,
        Some(list_part.trivia.clone()),
    );
    w.b.close(indent);
    w.b.close(open);
    id
}

/// Adds `( query )` hugging the query: `(` ends the line, the query is
/// indented under the line it opens on, and `)` starts a line at that
/// line's indentation.
pub(crate) fn hugged(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let (Some(first), Some(last)) = (parts.first(), parts.last()) else {
        return;
    };
    if parts.len() < 3
        || !first.el.is_token(SyntaxKind::OpeningRoundBracket)
        || !last.el.is_token(SyntaxKind::ClosingRoundBracket)
    {
        w.inline(node.kind(), &parts);
        return;
    }
    w.el(&first.el);
    let hug = w.b.open_hug();
    let indent = w.b.open_indent();
    for (k, part) in parts[1..parts.len() - 1].iter().enumerate() {
        let sep = if k == 0 {
            Sep::Hard
        } else {
            w.join(node.kind(), &parts[k].el, &part.el)
        };
        w.gap(part.trivia.clone(), sep, 0);
        w.el(&part.el);
    }
    w.b.close(indent);
    w.gap(last.trivia.clone(), Sep::Hard, 0);
    w.el(&last.el);
    w.b.close(hug);
    w.closed(w.range(node).1, CloserLine::Always);
}

/// Whether `el` starts with a token of `kind`.
pub(crate) fn starts_with(w: &Writer<'_>, el: &El, kind: TokenKind) -> bool {
    w.first_kind(el) == kind
}
