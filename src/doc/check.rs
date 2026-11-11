//! Checking that a document prints every token exactly once, in order.

use std::fmt;

use super::Doc;
use super::ir::Node;
use crate::Position;
use crate::lexer::TokenKind;

/// A document does not print the source's tokens exactly once each, in
/// order: a bug in the layout that built it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenCheckError {
    /// Where in the source the first token out of place is.
    pub position: Position,
    /// What is wrong.
    pub detail: String,
}

impl fmt::Display for TokenCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the document does not print the input's tokens once each, in order, at {}: {}",
            self.position, self.detail
        )
    }
}

impl std::error::Error for TokenCheckError {}

pub(super) fn check_tokens(doc: &Doc<'_>) -> Result<(), TokenCheckError> {
    let tokens = doc.tokens;
    let solid_from = |mut i: usize| {
        while tokens
            .get(i)
            .is_some_and(|t| t.kind() == TokenKind::Whitespace)
        {
            i += 1;
        }
        i
    };
    let mut next = 0;
    for node in &doc.nodes {
        let (first, last) = match *node {
            Node::Atom(atom) => (atom.token as usize, atom.token as usize),
            Node::Verbatim(v) => {
                let v = doc.verbatims[v as usize];
                (v.first as usize, v.last as usize)
            }
            _ => continue,
        };
        let expected = solid_from(next);
        if first != expected {
            return Err(misplaced(doc, expected, first));
        }
        next = last + 1;
    }
    let expected = solid_from(next);
    if expected < tokens.len() {
        return Err(TokenCheckError {
            position: Position::new(doc.source, tokens[expected].start()),
            detail: format!(
                "{} and every token after it are not printed",
                describe(doc, expected)
            ),
        });
    }
    Ok(())
}

fn misplaced(doc: &Doc<'_>, expected: usize, found: usize) -> TokenCheckError {
    let tokens = doc.tokens;
    let at = |i: usize| tokens.get(i).map_or(doc.source.len(), |t| t.start());
    let detail = if expected >= tokens.len() {
        format!("{} is printed after the last token", describe(doc, found))
    } else if found < expected {
        format!(
            "{} is printed again, or out of order, where {} comes next",
            describe(doc, found),
            describe(doc, expected)
        )
    } else {
        format!(
            "{} is not printed: the next token printed is {}",
            describe(doc, expected),
            describe(doc, found)
        )
    };
    TokenCheckError {
        position: Position::new(doc.source, at(expected.min(found))),
        detail,
    }
}

fn describe(doc: &Doc<'_>, index: usize) -> String {
    match doc.tokens.get(index) {
        Some(token) => {
            let text = token.text(doc.source);
            let excerpt: String = text.chars().take(40).collect();
            let more = if excerpt.len() < text.len() {
                "…"
            } else {
                ""
            };
            format!("token {index} ({:?} {excerpt:?}{more})", token.kind())
        }
        None => format!("token {index} (past the end)"),
    }
}
