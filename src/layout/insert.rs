//! `INSERT`: any `WITH` clause first, then the header line, the data as
//! written, and a query starting at the statement's indentation.
//!
//! The header line holds `INSERT INTO`, the target and its column list, then
//! `FROM INFILE`, `SETTINGS`, and `VALUES` or `FORMAT name`. It stays on one
//! line if it fits; otherwise each of those clauses starts a line of its
//! own at the statement's indentation, as ClickHouse prints `SETTINGS`, but
//! the first stays on the line of the column list's `)` when the list broke.
//!
//! The data is a token that keeps its bytes. After `FORMAT name` it starts
//! the next line at column 0 if a line break comes before it in the input,
//! and follows a space otherwise: ClickHouse skips the spaces and the line
//! break before it. The rows of `VALUES` (or `FORMAT Values`) follow a
//! space, or, where a line break comes before them in the input, keep the
//! whitespace before them as written, so that the first row keeps its
//! indentation, as do comments there; whitespace with more than one blank
//! line, `\r` or spaces at the end of a line gives a line break and one
//! blank line at most instead. What comes between the rows and the `;`
//! stays as written: `clickhouse-format` reads the rows to the end of their
//! line, and they can end in a line comment.

use super::query;
use super::tree::El;
use super::tree::Node;
use super::tree::Part;
use super::writer::Sep;
use super::writer::Writer;
use crate::doc::Gap;
use crate::doc::GroupId;
use crate::lexer::TokenKind;
use crate::syntax::SyntaxKind;

/// The body of an `INSERT` statement, an
/// [`InsertQuery`](SyntaxKind::InsertQuery).
pub(crate) fn statement(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let mut rest = &parts[..];
    if let Some((with, after)) = rest.split_first()
        && with.el.kind() == SyntaxKind::WithClause
    {
        w.el(&with.el);
        rest = after;
        if let Some(into) = rest.first() {
            w.gap(into.trivia.clone(), Sep::Hard, 1);
        }
    }
    let Some((into, after)) = rest.split_first() else {
        return;
    };
    rest = after;
    let header = w.b.open_group();
    let mut columns = match &into.el {
        El::Node(node) if node.kind() == SyntaxKind::InsertInto => target(w, node),
        el => {
            w.el(el);
            None
        }
    };
    let mut data = None;
    while let Some((part, after)) = rest.split_first() {
        if !matches!(
            part.el.kind(),
            SyntaxKind::InfileClause
                | SyntaxKind::SettingsClause
                | SyntaxKind::InsertValues
                | SyntaxKind::InsertFormat
        ) {
            break;
        }
        if let Some(columns) = columns.take() {
            w.gap(part.trivia.clone(), Sep::Nothing, 0);
            w.b.if_break(columns, Gap::Space, Gap::Line);
        } else {
            w.gap(part.trivia.clone(), Sep::Line, 0);
        }
        data = clause(w, part);
        rest = after;
    }
    w.b.close(header);
    if let Some((data, format)) = data {
        insert_data(w, &data, format);
    }
    for part in rest {
        w.gap(part.trivia.clone(), Sep::Hard, 1);
        match &part.el {
            El::Node(node) if node.kind() == SyntaxKind::InsertFormat => {
                if let Some((data, format)) = clause(w, part) {
                    insert_data(w, &data, format);
                }
            }
            El::Node(node) => query::query(w, node),
            el @ El::Token { .. } => w.el(el),
        }
    }
}

/// `INSERT INTO [TABLE] target [PARTITION BY …] [(columns)]` on one line, as
/// far as the column list allows: it stays on the line if it fits, and goes
/// one column per line otherwise. Returns the column list's group, if
/// there is a list.
fn target(w: &mut Writer<'_>, node: &Node) -> Option<GroupId> {
    let parts = w.parts(node);
    let kind = node.kind();
    let mut columns = None;
    for (k, part) in parts.iter().enumerate() {
        if k > 0 {
            let sep = w.join(kind, &parts[k - 1].el, &part.el);
            w.gap(part.trivia.clone(), sep, 0);
        }
        if part.el.kind() == SyntaxKind::InsertColumns {
            let open = w.b.open_group();
            columns = open.group();
            w.el(&part.el);
            w.b.close(open);
        } else {
            w.el(&part.el);
        }
    }
    columns
}

/// A clause of the header line. For `VALUES` and `FORMAT name`, only the
/// keywords and the name: the data, if there is one, is returned, with
/// whether it is in a format other than `VALUES`, to go after the header's
/// group, which a token that spans lines would break.
fn clause<'g>(w: &mut Writer<'_>, part: &Part<'g>) -> Option<(Part<'g>, bool)> {
    let node = match &part.el {
        El::Node(node)
            if matches!(
                node.kind(),
                SyntaxKind::InsertValues | SyntaxKind::InsertFormat
            ) =>
        {
            *node
        }
        el => {
            w.el(el);
            return None;
        }
    };
    let mut parts = w.parts(&node);
    let data = parts
        .last()
        .filter(|p| p.el.is_token(SyntaxKind::InsertData))
        .cloned();
    if data.is_some() {
        parts.pop();
    }
    w.inline(node.kind(), &parts);
    let format = node.kind() == SyntaxKind::InsertFormat
        && !parts
            .iter()
            .any(|p| p.el.kind() == SyntaxKind::Name && is_values_format(w, &p.el));
    data.map(|data| (data, format))
}

/// Whether `el`, a format's name, is `Values`, whose data runs to the `;`
/// as that of `VALUES` does.
fn is_values_format(w: &Writer<'_>, el: &El) -> bool {
    let El::Node(node) = el else {
        return false;
    };
    let (first, last) = w.range(node);
    first == last && matches!(w.text(first), "Values" | "\"Values\"" | "`Values`")
}

/// The data token `part`, after the keyword or name before it (see the
/// [module documentation](self)).
fn insert_data(w: &mut Writer<'_>, part: &Part, format: bool) {
    let El::Token { index, .. } = part.el else {
        w.el(&part.el);
        return;
    };
    let trivia = part.trivia.clone();
    let breaks = trivia.clone().any(|i| w.text(i).contains('\n'));
    if format {
        // Only spaces, tabs and one line break, which ClickHouse skips.
        if breaks {
            let root = w.b.open_at_root();
            w.gap(trivia, Sep::Hard, 0);
            w.el(&part.el);
            w.b.close(root);
        } else {
            w.gap(trivia, Sep::Space, 0);
            w.el(&part.el);
        }
        return;
    }
    let comment = trivia
        .clone()
        .any(|i| w.token_kind(i) != TokenKind::Whitespace);
    let after = index + 1;
    match w.b.tokens().get(after).map(|t| t.kind()) {
        Some(TokenKind::Semicolon) => {
            w.freeze(index);
            w.freeze(after);
        }
        Some(TokenKind::Whitespace)
            if w.b.tokens().get(after + 1).map(|t| t.kind()) == Some(TokenKind::Semicolon) =>
        {
            w.freeze(after);
        }
        _ => {}
    }
    let whitespace: Vec<usize> = trivia
        .clone()
        .filter(|&i| w.token_kind(i) == TokenKind::Whitespace)
        .collect();
    if !(breaks || comment) {
        w.gap(trivia, Sep::Space, 0);
    } else if whitespace.iter().all(|&i| plain_line_breaks(w.text(i))) {
        for i in whitespace {
            w.freeze(i);
        }
        w.gap(trivia, Sep::Space, 0);
    } else {
        w.gap(trivia, Sep::Hard, 1);
    }
    w.el(&part.el);
}

/// Whether `whitespace` is spaces between tokens on a line, or one line
/// break or two and then any indentation, as a formatted text has it: no
/// `\r`, nothing at the end of a line.
fn plain_line_breaks(whitespace: &str) -> bool {
    if !whitespace.contains('\n') {
        return !whitespace.contains('\r');
    }
    let indentation = whitespace.trim_start_matches('\n');
    (1..=2).contains(&(whitespace.len() - indentation.len())) && !indentation.contains(['\n', '\r'])
}

#[cfg(test)]
mod tests;
