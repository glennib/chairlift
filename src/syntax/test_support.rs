//! Helpers for grammar tests: parse a text and render the tree in forms that
//! make good assertions.

use std::fmt::Write as _;

use rowan::NodeOrToken;
use rowan::WalkEvent;

use crate::lexer::Mode;
use crate::split::split;
use crate::syntax::Entry;
use crate::syntax::SyntaxKind;
use crate::syntax::SyntaxNode;
use crate::syntax::debug_tree;
use crate::syntax::grammar::fallback;
use crate::syntax::parse_as;
use crate::syntax::parse_with;
use crate::syntax::parser::Parser;

/// The full tree of `source` parsed as `entry`, with ranges and every
/// token.
pub(crate) fn file_tree(source: &str, mode: Mode, entry: Entry) -> String {
    let parse = parse_as(source, mode, entry);
    assert_eq!(parse.syntax().text().to_string(), source, "lossless");
    debug_tree(&parse.syntax())
}

/// The body of the first statement of `source` parsed as `entry`, without
/// whitespace tokens or ranges: kinds, indented, and token texts.
pub(crate) fn tree(source: &str, entry: Entry) -> String {
    let parse = parse_as(source, Mode::Jinja, entry);
    assert_eq!(parse.syntax().text().to_string(), source, "lossless");
    let body = parse
        .syntax()
        .children()
        .find(|n| n.kind() == SyntaxKind::Statement)
        .and_then(|s| s.first_child());
    body.map_or_else(|| "(no statement)\n".to_owned(), |b| compact(&b))
}

/// The expression `source` as [`tree`] renders it.
pub(crate) fn expr(source: &str) -> String {
    tree(source, Entry::Expression)
}

/// The first statement of `source` parsed by the grammar function `parse`
/// inside the statement fallback, as [`tree`] renders it: the body if the
/// statement has one node, every child otherwise.
pub(crate) fn grammar(source: &str, parse: fn(&mut Parser<'_>) -> Option<()>) -> String {
    let split = split(source, Mode::Jinja);
    let parse = parse_with(source, &split, &mut |p| {
        if !p.at_end() {
            fallback::statement(p, parse);
        }
    });
    assert_eq!(parse.syntax().text().to_string(), source, "lossless");
    let Some(statement) = parse
        .syntax()
        .children()
        .find(|n| n.kind() == SyntaxKind::Statement)
    else {
        return "(no statement)\n".to_owned();
    };
    statement.children().map(|child| compact(&child)).collect()
}

/// The re-cased text of `source` parsed as `entry`: what the keyword slots
/// are.
pub(crate) fn recased(source: &str, entry: Entry) -> String {
    parse_as(source, Mode::Jinja, entry).recased()
}

/// `node` without whitespace tokens or ranges.
pub(crate) fn compact(node: &SyntaxNode) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for event in node.preorder_with_tokens() {
        match event {
            WalkEvent::Enter(element) => {
                if element.kind() == SyntaxKind::Whitespace {
                    continue;
                }
                out.extend(std::iter::repeat_n("  ", depth));
                out.push_str(element.kind().name());
                if let NodeOrToken::Token(token) = &element {
                    let _ = write!(out, " {:?}", token.text());
                }
                out.push('\n');
                if element.as_node().is_some() {
                    depth += 1;
                }
            }
            WalkEvent::Leave(element) => {
                if element.as_node().is_some() {
                    depth -= 1;
                }
            }
        }
    }
    out
}

/// The expression `source`, with every operator node in parentheses and its
/// tokens separated by spaces: `a + b * c` is `(a + (b * c))`. The result is
/// ClickHouse syntax with the same meaning, so `EXPLAIN AST` of both must
/// agree if the parser's precedence is ClickHouse's.
pub(crate) fn parenthesized(source: &str) -> String {
    let parse = parse_as(source, Mode::Sql, Entry::Expression);
    let body = parse
        .syntax()
        .children()
        .find(|n| n.kind() == SyntaxKind::Statement)
        .and_then(|s| s.first_child());
    body.map_or_else(String::new, |b| render(&b))
}

fn render(node: &SyntaxNode) -> String {
    if node.kind() == SyntaxKind::BinaryExpr {
        return render_binary(node);
    }
    let parts: Vec<String> = node
        .children_with_tokens()
        .filter(|e| !e.kind().is_trivia())
        .map(|e| match e {
            NodeOrToken::Node(n) => render(&n),
            NodeOrToken::Token(t) => t.text().to_owned(),
        })
        .collect();
    let joined = parts.join(" ");
    let negative_literal = node.kind() == SyntaxKind::PrefixExpr
        && node
            .children()
            .next()
            .is_some_and(|c| c.kind() == SyntaxKind::Literal);
    match node.kind() {
        // Its text is what ClickHouse reads; parentheses would change it.
        SyntaxKind::LiteralAsText => node.text().to_string(),
        SyntaxKind::PrefixExpr if negative_literal => joined,
        SyntaxKind::AndExpr
        | SyntaxKind::OrExpr
        | SyntaxKind::PrefixExpr
        | SyntaxKind::PostfixExpr
        | SyntaxKind::BetweenExpr
        | SyntaxKind::TernaryExpr
        | SyntaxKind::LambdaExpr
        | SyntaxKind::CastExpr
        | SyntaxKind::IndexExpr
        | SyntaxKind::TupleAccessExpr
        | SyntaxKind::IntervalExpr => format!("({joined})"),
        _ => joined,
    }
}

/// A [`BinaryExpr`](SyntaxKind::BinaryExpr), which may hold a run of
/// operators of one priority, with each operator in parentheses from the
/// left: `a - b + c` is `((a - b) + c)`.
fn render_binary(node: &SyntaxNode) -> String {
    let mut acc: Option<String> = None;
    let mut ops: Vec<String> = Vec::new();
    let mut escape = false;
    for element in node
        .children_with_tokens()
        .filter(|e| !e.kind().is_trivia())
    {
        match element {
            NodeOrToken::Token(t) if t.text().eq_ignore_ascii_case("ESCAPE") && ops.is_empty() => {
                // `ESCAPE 'c'` belongs to the operator just applied.
                escape = true;
            }
            NodeOrToken::Token(t) => ops.push(t.text().to_owned()),
            NodeOrToken::Node(n) => {
                let operand = render(&n);
                acc = Some(match acc.take() {
                    None => operand,
                    Some(lhs) if escape => {
                        escape = false;
                        let open = lhs.strip_suffix(')').unwrap_or(&lhs);
                        format!("{open} ESCAPE {operand})")
                    }
                    Some(lhs) => {
                        format!("({lhs} {} {operand})", std::mem::take(&mut ops).join(" "))
                    }
                });
            }
        }
    }
    acc.unwrap_or_default()
}
