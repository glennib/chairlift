//! Expressions: operators, conditions, `CASE`, lambdas and the rest.
//!
//! Calls, tuples, arrays and the other bracketed lists are
//! [`shapes::bracketed`](super::shapes::bracketed); most other expression
//! nodes are laid out by the generic rule, on one line with ClickHouse's
//! spacing ([`Writer::join`]).

use super::tree::El;
use super::tree::Node;
use super::tree::Part;
use super::writer::CloserLine;
use super::writer::Sep;
use super::writer::Writer;
use crate::syntax::SyntaxKind;

/// A binary expression: a run of operators of one priority, grouped from
/// the left, each breaking before its operator, one level in, when what
/// it joins does not fit: `group(group(a indent(line + b)) indent(line - c))`.
pub(crate) fn binary(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let Some(segments) = segments(w, &parts) else {
        w.generic(node);
        return;
    };
    let kind = node.kind();
    let Some((first, rest)) = segments.split_first() else {
        return;
    };
    let mut opens: Vec<_> = rest.iter().map(|_| w.b.open_group()).collect();
    let first = &parts[first.operand.clone()];
    w.inline(kind, first);
    let mut closer = closer_line(w, first);
    for (k, segment) in rest.iter().enumerate() {
        let ops = &parts[segment.op.clone()];
        let operand = &parts[segment.operand.clone()];
        let indent = w.b.open_indent();
        let trivia = ops[0].trivia.clone();
        let mut on_closer_line = None;
        if hugs_after(w, ops, operand) || multiline(w, operand) {
            w.gap(trivia, Sep::Space, 0);
        } else if let Some(closer) = closer.filter(|_| w.only_whitespace(&trivia)) {
            on_closer_line = Some(w.open_after_closer(closer));
            w.gap(trivia, Sep::Nothing, 0);
        } else {
            w.gap(trivia, Sep::Line, 0);
        }
        w.inline(kind, ops);
        if let Some(open) = on_closer_line {
            w.b.close(open);
        }
        w.gap(operand[0].trivia.clone(), Sep::Space, 0);
        w.inline(kind, operand);
        if k + 1 < rest.len() {
            closer = closer_line(w, operand);
        }
        w.b.close(indent);
        if let Some(open) = opens.pop() {
            w.b.close(open);
        }
    }
}

/// When the last line of `operand`, just added, starts with a closer with
/// no place to break after it: a subquery's `)`, or the `)` or `END` of a
/// list or `CASE` that breaks. The operator after it then stays on that
/// line if it fits there with the start of its operand: `) > 0`, `)) = 0`,
/// `) = g(`. After a closer followed by more code that can break, or one
/// that ends a line of other code, `(a\n    || b) = c`, the operator breaks
/// as anywhere else rather than hide in the middle of a line. After an
/// operand that holds a token spanning lines, the operator stays on the
/// operand's last line in the same way.
fn closer_line(w: &Writer<'_>, operand: &[Part]) -> Option<CloserLine> {
    if multiline(w, operand) {
        return Some(CloserLine::Always);
    }
    let first = match &operand.first()?.el {
        El::Node(node) => w.range(node).0,
        El::Token { index, .. } => *index,
    };
    w.closer_line(first)
}

/// Whether `operand` holds a token that spans lines, such as a string: it
/// breaks every group around it, which must not move the operator before it
/// to a line of its own.
fn multiline(w: &Writer<'_>, operand: &[Part]) -> bool {
    operand.iter().any(|part| match &part.el {
        El::Node(node) => w.has(node, super::tree::flag::MULTILINE),
        El::Token { index, .. } => w.text(*index).contains('\n'),
    })
}

/// One operand of a binary expression and the operator before it, as
/// ranges of its parts.
struct Segment {
    op: std::ops::Range<usize>,
    operand: std::ops::Range<usize>,
}

/// Splits a binary expression's children into operands and the operators
/// between them; `ESCAPE 'c'` after an operand belongs to it. `None` for a
/// shape that is not an operand followed by operators and operands.
fn segments(w: &Writer<'_>, parts: &[Part]) -> Option<Vec<Segment>> {
    let is_node = |i: usize| matches!(parts[i].el, El::Node(_));
    if parts.is_empty() || !is_node(0) {
        return None;
    }
    let mut segments: Vec<Segment> = vec![Segment {
        op: 0..0,
        operand: 0..1,
    }];
    let mut i = 1;
    while i < parts.len() {
        let op_start = i;
        while i < parts.len() && !is_node(i) {
            i += 1;
        }
        if i == parts.len() || i == op_start {
            return None;
        }
        let escape = match parts[op_start].el {
            El::Token { index, .. } => w.text(index).eq_ignore_ascii_case("escape"),
            El::Node(_) => false,
        };
        match segments.last_mut() {
            Some(last) if escape => last.operand.end = i + 1,
            _ => segments.push(Segment {
                op: op_start..i,
                operand: i..i + 1,
            }),
        }
        i += 1;
    }
    Some(segments)
}

/// Whether the operand after `ops` hugs its line, so that the operator
/// stays on the line before it: a subquery, `ANY (…)`, and the list after
/// `IN`.
fn hugs_after(w: &Writer<'_>, ops: &[Part], operand: &[Part]) -> bool {
    let Some(El::Node(first)) = operand.first().map(|p| &p.el) else {
        return false;
    };
    match first.kind() {
        SyntaxKind::Subquery | SyntaxKind::QuantifiedExpr => true,
        SyntaxKind::TupleExpr | SyntaxKind::ArrayExpr => ops.iter().any(|op| match op.el {
            El::Token { index, .. } => w.text(index).eq_ignore_ascii_case("in"),
            El::Node(_) => false,
        }),
        _ => false,
    }
}

/// A run of `AND`s or `OR`s, hanging: the first operand where the run
/// starts, each further one on a line of its own starting with the
/// operator, one level in, unless they all fit on one line.
pub(crate) fn hanging(w: &mut Writer<'_>, node: &Node) {
    let open = w.b.open_group();
    conditions(w, node, true);
    w.b.close(open);
}

/// The operands of a run of `AND`s or `OR`s, the first where the run
/// starts and each further one after a line, with its operator; indented
/// one level if `indent`, otherwise in the current scope (as `ON` lays out
/// its conditions under it).
pub(crate) fn conditions(w: &mut Writer<'_>, node: &Node, indent: bool) {
    let parts = w.parts(node);
    let Some((first, rest)) = parts.split_first() else {
        return;
    };
    // The first operand is in the scope too: an `OR` group in brackets
    // there breaks one level deeper than the `AND`s after it.
    let scope = indent.then(|| w.b.open_indent());
    w.el(&first.el);
    for part in rest {
        // The operators are tokens, the operands nodes.
        let sep = match part.el {
            El::Token { .. } => Sep::Line,
            El::Node(_) => Sep::Space,
        };
        w.gap(part.trivia.clone(), sep, 0);
        w.el(&part.el);
    }
    if let Some(scope) = scope {
        w.b.close(scope);
    }
}

/// `CASE`: on one line if it fits, otherwise each `WHEN … THEN …` and the
/// `ELSE` on a line of its own, one level in, and `END` at the
/// indentation of `CASE`'s line.
pub(crate) fn case(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    let arm = |p: &Part| matches!(p.el.kind(), SyntaxKind::CaseWhen | SyntaxKind::CaseElse);
    let Some(first_arm) = parts.iter().position(arm) else {
        w.generic(node);
        return;
    };
    let end = parts.iter().rposition(arm).map_or(parts.len(), |i| i + 1);
    let open = w.b.open_group();
    let group = open.group();
    // `END` goes back to the indentation of the line `CASE` is on.
    let hug = w.b.open_hug();
    w.inline(kind, &parts[..first_arm]);
    let indent = w.b.open_indent();
    for part in &parts[first_arm..end] {
        w.gap(part.trivia.clone(), Sep::Line, 0);
        w.el(&part.el);
    }
    w.b.close(indent);
    for part in &parts[end..] {
        w.gap(part.trivia.clone(), Sep::Line, 0);
        w.el(&part.el);
    }
    w.b.close(hug);
    w.b.close(open);
    if let Some(El::Token { index, .. }) = parts.get(end).map(|part| &part.el)
        && end + 1 == parts.len()
        && w.text(*index).eq_ignore_ascii_case("end")
        && let Some(group) = group
    {
        w.closed(*index, CloserLine::IfBroken(group));
    }
}

/// A lambda: one line if it fits, otherwise a line break after `->` and
/// the body one level in.
pub(crate) fn lambda(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let Some(arrow) = parts.iter().position(|p| p.el.is_token(SyntaxKind::Arrow)) else {
        w.generic(node);
        return;
    };
    let kind = node.kind();
    let open = w.b.open_group();
    // The body is one level in from the line the lambda starts on.
    let hug = w.b.open_hug();
    w.inline(kind, &parts[..=arrow]);
    let indent = w.b.open_indent();
    if let Some((body, rest)) = parts[arrow + 1..].split_first() {
        w.gap(body.trivia.clone(), Sep::Line, 0);
        w.el(&body.el);
        let mut prev = &body.el;
        for part in rest {
            let sep = w.join(kind, prev, &part.el);
            w.gap(part.trivia.clone(), sep, 0);
            w.el(&part.el);
            prev = &part.el;
        }
    }
    w.b.close(indent);
    w.b.close(hug);
    w.b.close(open);
}

/// `c ? a : b`: one line if it fits, otherwise `? a` and `: b` on lines of
/// their own, one level in.
pub(crate) fn ternary(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    let open = w.b.open_group();
    let indent = w.b.open_indent();
    for (k, part) in parts.iter().enumerate() {
        if k > 0 {
            let sep = if matches!(
                part.el,
                El::Token {
                    kind: SyntaxKind::QuestionMark | SyntaxKind::Colon,
                    ..
                }
            ) {
                Sep::Line
            } else {
                w.join(kind, &parts[k - 1].el, &part.el)
            };
            w.gap(part.trivia.clone(), sep, 0);
        }
        w.el(&part.el);
    }
    w.b.close(indent);
    w.b.close(open);
}

/// `x [NOT] BETWEEN a AND b`: on one line if it fits, otherwise a line
/// break before `[NOT] BETWEEN`, one level in. Its `AND` is never a place to
/// break, so that it cannot read as one of a run of conditions.
pub(crate) fn between(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    let Some(at) = parts.iter().position(|p| matches!(p.el, El::Token { .. })) else {
        w.generic(node);
        return;
    };
    let open = w.b.open_group();
    w.inline(kind, &parts[..at]);
    let indent = w.b.open_indent();
    for (k, part) in parts.iter().enumerate().skip(at) {
        let sep = if k == at {
            Sep::Line
        } else {
            w.join(kind, &parts[k - 1].el, &part.el)
        };
        w.gap(part.trivia.clone(), sep, 0);
        w.el(&part.el);
    }
    w.b.close(indent);
    w.b.close(open);
}

/// `x::Type`: the type on one line, never broken inside, as the proposal
/// has casts.
pub(crate) fn cast(w: &mut Writer<'_>, node: &Node) {
    let parts = w.parts(node);
    let kind = node.kind();
    let Some(colons) = parts
        .iter()
        .position(|p| p.el.is_token(SyntaxKind::DoubleColon))
    else {
        w.generic(node);
        return;
    };
    w.inline(kind, &parts[..=colons]);
    w.on_one_line(|w| {
        for (k, part) in parts.iter().enumerate().skip(colons + 1) {
            let sep = w.join(kind, &parts[k - 1].el, &part.el);
            w.gap(part.trivia.clone(), sep, 0);
            w.el(&part.el);
        }
    });
}

/// How deep data types nest before the inner ones go on one line.
const TYPE_DEPTH: u32 = 4;

/// A data type: by the generic rule, and on one line when it is nested in
/// [`TYPE_DEPTH`] others, so that a type nested a hundred deep does not
/// indent without bound.
pub(crate) fn data_type(w: &mut Writer<'_>, node: &Node) {
    w.type_depth += 1;
    if w.type_depth > TYPE_DEPTH {
        w.on_one_line(|w| w.generic(node));
    } else {
        w.generic(node);
    }
    w.type_depth -= 1;
}

/// Whether `node` is a run of `AND`s or `OR`s.
pub(crate) fn is_condition_run(node: &Node) -> bool {
    matches!(node.kind(), SyntaxKind::AndExpr | SyntaxKind::OrExpr)
}
