//! The file: statements, what lies between them, and which statements are
//! laid out and which kept as written.

use std::ops::Range;

use super::Decision;
use super::Kept;
use super::Statement;
use super::tree::El;
use super::tree::Node;
use super::tree::Part;
use super::tree::flag;
use super::writer::Sep;
use super::writer::Writer;
use crate::doc::GroupId;
use crate::lexer::TokenKind;
use crate::syntax::SyntaxKind;

/// A statement's layout rule.
pub(crate) type Rule = fn(&mut Writer<'_>, &Node);

/// The layout rule for a statement whose body is of `kind`; `None` keeps
/// such statements as written.
pub(crate) fn rule_for(kind: SyntaxKind) -> Option<Rule> {
    match kind {
        SyntaxKind::SelectQuery | SyntaxKind::UnionQuery => Some(super::query::query),
        SyntaxKind::CreateTable | SyntaxKind::CreateMaterializedView => {
            Some(super::create::statement)
        }
        SyntaxKind::InsertQuery => Some(super::insert::statement),
        _ => None,
    }
}

/// Whether to lay out `statement`, a [`Statement`](SyntaxKind::Statement)
/// node, or keep it as written, and why.
pub(crate) fn decide(w: &Writer<'_>, statement: &Node, body: Option<&Node>) -> Decision {
    let Some(body) = body else {
        return Decision::Formatted;
    };
    if body.kind() == SyntaxKind::VerbatimStatement {
        return Decision::Kept(Kept::NotParsed);
    }
    if rule_for(body.kind()).is_none() {
        return Decision::Kept(Kept::NoRule);
    }
    let flags = w.info(statement).flags;
    if flags & flag::RENDER_JOINED != 0 {
        Decision::Kept(Kept::RenderJoined)
    } else if flags & flag::TEMPLATE_DATA != 0 {
        Decision::Kept(Kept::TemplateData)
    } else if flags & flag::HIDDEN_TAG != 0 {
        Decision::Kept(Kept::HiddenTag)
    } else if flags & flag::TAG_PLACE != 0 {
        Decision::Kept(Kept::TagPlace)
    } else if flags & flag::TAG_TOUCHING != 0 {
        Decision::Kept(Kept::TagTouching)
    } else {
        Decision::Formatted
    }
}

/// A statement with what was decided for it.
pub(crate) struct Planned<'g> {
    pub(crate) node: Node<'g>,
    pub(crate) part: usize,
    pub(crate) decision: Decision,
    /// Only a `;`.
    pub(crate) empty: bool,
}

/// Whether `statement`, a [`Statement`](SyntaxKind::Statement) node, ends
/// with the rows of `INSERT … VALUES`, before any `;`.
fn ends_with_rows(w: &Writer<'_>, statement: &Node) -> bool {
    let (first, last) = w.range(statement);
    (first..=last)
        .rev()
        .find(|&i| {
            !matches!(
                w.token_kind(i),
                TokenKind::Whitespace | TokenKind::Semicolon
            )
        })
        .is_some_and(|i| w.token_kind(i) == TokenKind::InsertData)
}

/// Lays out the root: the statements in `plan`, each in a group of its own,
/// one per line, with the comments, tags and blank lines between them.
pub(crate) fn root(w: &mut Writer<'_>, root: &Node, plan: &[Planned]) {
    let (parts, tail) = w.parts_and_tail(root);
    // The group of the last statement that is not empty, and whether what
    // trails its last line stays as written (see `freeze_trailing`).
    let mut previous: Option<(GroupId, bool)> = None;
    for planned in plan {
        let part = &parts[planned.part];
        let kept = planned.decision != Decision::Formatted;
        let same_line = || {
            !part
                .trivia
                .clone()
                .any(|i| w.is_tag(i) || w.text(i).contains('\n'))
        };
        if planned.empty
            && same_line()
            && let Some((_, previous_kept)) = previous
        {
            // `;;`: the empty statement stays on the line of the one before.
            freeze_trailing(w, part.trivia.clone(), previous_kept);
            w.gap(part.trivia.clone(), Sep::Nothing, 0);
            statement(w, &planned.node, planned.decision);
            continue;
        }
        let id = w.reserve_group();
        match previous {
            None => leading(w, part.trivia.clone()),
            Some((previous, previous_kept)) => {
                between(w, part.trivia.clone(), previous, previous_kept, id);
            }
        }
        w.frozen_before(w.range(&planned.node).0);
        let open = w.b.open_reserved_group(id);
        statement(w, &planned.node, planned.decision);
        w.b.close(open);
        previous = Some((id, kept || ends_with_rows(w, &planned.node)));
    }
    // Significant children of the root other than statements do not exist;
    // trivia after the last statement, or of a text without statements.
    match previous {
        None => leading(w, tail.clone()),
        Some((previous, previous_kept)) => trailing(w, tail, previous, previous_kept),
    }
}

/// Keeps the whitespace before comments that trail a statement kept as
/// written, or one that ends with `INSERT … VALUES` rows, on its last line
/// as it is, as the rest of the line is: tools that read the rows to the end
/// of the line, as `clickhouse-format` does, see it.
fn freeze_trailing(w: &mut Writer<'_>, trivia: Range<usize>, kept: bool) {
    if !kept {
        return;
    }
    for i in trivia {
        match w.token_kind(i) {
            TokenKind::Whitespace if w.text(i).contains('\n') => break,
            TokenKind::Whitespace => w.freeze(i),
            TokenKind::LineComment => break,
            _ => {}
        }
    }
}

/// A statement: laid out by its rule, or as written.
fn statement(w: &mut Writer<'_>, node: &Node, decision: Decision) {
    if decision != Decision::Formatted {
        let (first, last) = w.range(node);
        w.as_written(first, last);
        return;
    }
    let parts = w.parts(node);
    for (k, part) in parts.iter().enumerate() {
        if k > 0 {
            w.gap(part.trivia.clone(), Sep::Nothing, 0);
        }
        match &part.el {
            El::Node(body) => match rule_for(body.kind()) {
                Some(rule) => rule(w, body),
                None => w.verbatim(body),
            },
            el @ El::Token { .. } => w.el(el),
        }
    }
}

/// What comes before the first statement: comments and tags on lines of
/// their own, keeping the blank line, one at most, written between them and
/// before the statement: a header comment keeps the 0 or 1 its author gave it.
fn leading(w: &mut Writer<'_>, trivia: Range<usize>) {
    w.between_statements = true;
    w.gap(trivia, Sep::Nothing, 1);
    w.between_statements = false;
}

/// What comes between two statements: comments trailing the first stay on
/// its last line; the rest go on lines of their own at column 0. Blank
/// lines are kept up to two, and there is one before and after a statement
/// that spans lines, unless a block tag is next to it.
fn between(
    w: &mut Writer<'_>,
    trivia: Range<usize>,
    previous: GroupId,
    previous_kept: bool,
    next: GroupId,
) {
    let tags = trivia.clone().any(|i| w.is_tag(i));
    let (trailing, rest) = split_trailing(w, trivia);
    freeze_trailing(w, trailing.clone(), previous_kept);
    w.gap(trailing, Sep::Nothing, 0);
    w.b.hard_line();
    if !tags {
        w.b.blank_line_if_broken(previous);
        w.b.blank_line_if_broken(next);
    }
    w.between_statements = true;
    w.gap(rest, Sep::Nothing, 2);
    w.between_statements = false;
}

/// What comes after the last statement.
fn trailing(w: &mut Writer<'_>, trivia: Range<usize>, previous: GroupId, previous_kept: bool) {
    let tags = trivia.clone().any(|i| w.is_tag(i));
    let (trailing, rest) = split_trailing(w, trivia);
    freeze_trailing(w, trailing.clone(), previous_kept);
    w.gap(trailing, Sep::Nothing, 0);
    if rest
        .clone()
        .any(|i| w.token_kind(i) != TokenKind::Whitespace)
    {
        w.b.hard_line();
        if !tags {
            w.b.blank_line_if_broken(previous);
        }
        w.gap(rest, Sep::Nothing, 2);
    }
}

/// Splits trivia after a statement into the comments that trail its last
/// line and the rest.
fn split_trailing(w: &Writer<'_>, trivia: Range<usize>) -> (Range<usize>, Range<usize>) {
    let mut end = trivia.start;
    for i in trivia.clone() {
        match w.token_kind(i) {
            TokenKind::Whitespace => {}
            TokenKind::LineComment if w.newlines_before(i) == 0 => {
                end = i + 1;
                break;
            }
            TokenKind::BlockComment | TokenKind::JinjaComment if w.newlines_before(i) == 0 => {
                end = i + 1;
            }
            _ => break,
        }
    }
    (trivia.start..end, end..trivia.end)
}

/// The statements among the root's parts, with their decisions.
pub(crate) fn plan<'g>(
    w: &Writer<'_>,
    parts: &[Part<'g>],
    keep_all: Option<Kept>,
) -> Vec<Planned<'g>> {
    parts
        .iter()
        .enumerate()
        .filter_map(|(i, part)| {
            let node = part.el.node()?;
            if node.kind() != SyntaxKind::Statement {
                return None;
            }
            let body = w.parts(&node).into_iter().find_map(|p| p.el.node());
            let empty = body.is_none();
            let decision = match decide(w, &node, body.as_ref()) {
                Decision::Formatted => match keep_all {
                    Some(kept) if body.is_some() => Decision::Kept(kept),
                    _ => Decision::Formatted,
                },
                kept @ Decision::Kept(_) => kept,
            };
            Some(Planned {
                node,
                part: i,
                decision,
                empty,
            })
        })
        .collect()
}

/// The statements of `plan` as reported.
pub(crate) fn report(w: &Writer<'_>, plan: &[Planned]) -> Vec<Statement> {
    plan.iter()
        .map(|p| {
            let (first, last) = w.range(&p.node);
            Statement {
                tokens: first..last + 1,
                decision: p.decision,
            }
        })
        .collect()
}
