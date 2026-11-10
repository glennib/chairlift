//! The fallback ladder: combinators that try a parse and, if it fails, keep
//! the region verbatim instead, at the level of a list item, a clause or a
//! statement.
//!
//! A grammar function signals failure by returning `None`, and may leave
//! half-written events behind when it does: the combinators roll back to
//! where they started, so only they need to clean up. Every grammar function
//! that can fail therefore runs inside one of them, and the statement
//! combinator is the outermost.
//!
//! The verbatim scans count brackets (`(`, `[`, `{` against `)`, `]`, `}`)
//! and never consume a closer that would leave the region unbalanced, which
//! belongs to whatever encloses it. Trivia is not looked at: comments and
//! Jinja block tags are never tokens the parser sees.

use crate::lexer::TokenKind;
use crate::syntax::FallbackLevel;
use crate::syntax::SyntaxKind;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Says whether the parser is where a list, or a clause, ends: at a clause
/// keyword that follows, for a list that is not in brackets.
pub(crate) type Stop<'a> = &'a dyn Fn(&Parser<'_>) -> bool;

/// A stop that never stops: the end of the statement or a closing bracket
/// ends the region.
pub(crate) fn never(_: &Parser<'_>) -> bool {
    false
}

/// Parses a statement with `body`. If it fails, or leaves tokens unparsed,
/// the statement is rolled back and kept verbatim as a whole, and the reason
/// is recorded.
pub(crate) fn statement(p: &mut Parser<'_>, body: impl FnOnce(&mut Parser<'_>) -> Option<()>) {
    let checkpoint = p.checkpoint();
    if body(p).is_some() {
        if p.at_end() {
            return;
        }
        p.expected::<()>("the end of the statement");
    }
    p.rollback(checkpoint);
    verbatim_statement(p);
    p.record_fallback(FallbackLevel::Statement, checkpoint);
}

/// The rest of the statement, verbatim. This is how statements of kinds the
/// parser does not know are kept, and records no fallback.
pub(crate) fn verbatim_statement(p: &mut Parser<'_>) {
    if p.at_end() {
        return;
    }
    let m = p.start();
    while !p.at_end() {
        p.bump();
    }
    m.complete(p, SyntaxKind::VerbatimStatement);
}

/// Parses a clause with `body`, which starts at the clause's keyword.
///
/// The clause stands if `body` succeeds and is followed by the end of the
/// statement, a closing bracket, or a clause keyword (`at_next_clause`).
/// Otherwise the clause is rolled back and kept verbatim, from its keyword
/// to the next clause keyword outside brackets, and the reason is recorded.
/// Returns whether the clause parsed.
///
/// Since clauses are tried one at a time, they may come in any order and
/// repeat, which is what lets `{% if %} WHERE a {% else %} WHERE b
/// {% endif %}` parse as two clauses.
#[allow(dead_code, reason = "for the statement grammar that builds on it")]
pub(crate) fn clause(
    p: &mut Parser<'_>,
    at_next_clause: Stop<'_>,
    body: impl FnOnce(&mut Parser<'_>) -> Option<()>,
) -> bool {
    if p.at_end() {
        return false;
    }
    let checkpoint = p.checkpoint();
    let start = p.position();
    if body(p).is_some() && p.position() > start {
        if p.at_end() || p.at_closer() || at_next_clause(p) {
            return true;
        }
        p.expected::<()>("the next clause");
    }
    p.rollback(checkpoint);
    let m = p.start();
    p.bump();
    skip(p, at_next_clause);
    m.complete(p, SyntaxKind::VerbatimClause);
    p.record_fallback(FallbackLevel::Clause, checkpoint);
    false
}

/// Parses one item of a list with `item`.
///
/// The item stands if `item` succeeds and is followed by `,`, the end of the
/// list (a closing bracket, or `ends`), or the end of the statement.
/// Otherwise it is rolled back and kept verbatim up to the next `,` or the
/// end of the list outside brackets, and the reason is recorded. An item
/// that fails at a `,` or the end of the list consumes nothing.
pub(crate) fn list_item(
    p: &mut Parser<'_>,
    ends: Stop<'_>,
    item: impl FnOnce(&mut Parser<'_>) -> Option<()>,
) {
    let checkpoint = p.checkpoint();
    if item(p).is_some() {
        if p.at(TokenKind::Comma) || p.at_end() || p.at_closer() || ends(p) {
            return;
        }
        p.expected::<()>("`,` or the end of the list");
    }
    p.rollback(checkpoint);
    let m = p.start();
    if skip(p, &|p: &Parser<'_>| p.at(TokenKind::Comma) || ends(p)) {
        m.complete(p, SyntaxKind::VerbatimItem);
        p.record_fallback(FallbackLevel::Item, checkpoint);
    } else {
        m.abandon(p);
        p.clear_expected();
    }
}

/// How many items and commas a list had.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ListShape {
    pub(crate) items: usize,
    pub(crate) commas: usize,
}

/// Items separated by commas, each parsed by `item` with [`list_item`]'s
/// fallback, up to the end of the list: a closing bracket, `ends`, or the
/// end of the statement. A trailing comma and empty items are accepted.
/// The caller opens and closes the list's node and its brackets.
pub(crate) fn comma_separated(
    p: &mut Parser<'_>,
    ends: Stop<'_>,
    item: &mut dyn FnMut(&mut Parser<'_>) -> Option<()>,
) -> ListShape {
    let mut shape = ListShape::default();
    loop {
        if p.at_end() || p.at_closer() || ends(p) {
            break;
        }
        if p.at(TokenKind::Comma) {
            p.bump();
            shape.commas += 1;
            continue;
        }
        let before = p.position();
        list_item(p, ends, &mut *item);
        if p.position() == before {
            break;
        }
        shape.items += 1;
        if p.at(TokenKind::Comma) {
            p.bump();
            shape.commas += 1;
        } else {
            break;
        }
    }
    shape
}

/// `open`, items separated by commas, `close`, as a node of `kind`: a
/// bracketed list. Fails if the closing bracket is missing.
#[allow(dead_code, reason = "for the statement grammar that builds on it")]
pub(crate) fn delimited(
    p: &mut Parser<'_>,
    kind: SyntaxKind,
    open: TokenKind,
    close: TokenKind,
    item: &mut dyn FnMut(&mut Parser<'_>) -> Option<()>,
) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect(open)?;
    comma_separated(p, &never, item);
    p.expect(close)?;
    Some(m.complete(p, kind))
}

/// A bracketed group kept as it is, as an [`Opaque`](SyntaxKind::Opaque)
/// node: from the opening bracket at the current token to its match. Fails
/// if the current token is not an opening bracket or the group is not
/// closed in the statement.
pub(crate) fn opaque_group(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !matches!(
        p.nth(0),
        Some(
            TokenKind::OpeningRoundBracket
                | TokenKind::OpeningSquareBracket
                | TokenKind::OpeningCurlyBrace
        )
    ) {
        return p.expected("`(`");
    }
    let m = p.start();
    let mut depth = 0usize;
    while let Some(kind) = p.nth(0) {
        if is_opener(kind) {
            depth += 1;
        } else if is_closer(kind) {
            depth -= 1;
        }
        p.bump();
        if depth == 0 {
            return Some(m.complete(p, SyntaxKind::Opaque));
        }
    }
    p.expected("a closing bracket")
}

/// Tokens up to `stop` outside brackets, an unmatched closing bracket, or
/// the end of the statement, kept as they are, as an
/// [`Opaque`](SyntaxKind::Opaque) node. Fails if that is nothing.
pub(crate) fn opaque_until(p: &mut Parser<'_>, stop: Stop<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    if skip(p, stop) {
        Some(m.complete(p, SyntaxKind::Opaque))
    } else {
        m.abandon(p);
        p.expected("something to keep as written")
    }
}

/// Consumes tokens as they are up to `stop` outside brackets, an unmatched
/// closing bracket, or the end of the statement. Returns whether it consumed
/// any.
pub(crate) fn skip(p: &mut Parser<'_>, stop: Stop<'_>) -> bool {
    let mut depth = 0usize;
    let mut consumed = false;
    while let Some(kind) = p.nth(0) {
        if depth == 0 && (is_closer(kind) || stop(p)) {
            break;
        }
        if is_opener(kind) {
            depth += 1;
        } else if is_closer(kind) {
            depth -= 1;
        }
        p.bump();
        consumed = true;
    }
    consumed
}

pub(crate) fn is_opener(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::OpeningRoundBracket
            | TokenKind::OpeningSquareBracket
            | TokenKind::OpeningCurlyBrace
    )
}

pub(crate) fn is_closer(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::ClosingRoundBracket
            | TokenKind::ClosingSquareBracket
            | TokenKind::ClosingCurlyBrace
    )
}

#[cfg(test)]
mod tests;
