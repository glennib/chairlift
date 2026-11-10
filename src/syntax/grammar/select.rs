//! `SELECT` queries.
//!
//! Not parsed yet: a query statement is kept verbatim, and a query anywhere
//! else is [`Opaque`](SyntaxKind::Opaque). The functions here are the seams
//! the rest of the grammar calls:
//!
//! - [`at_statement`] and [`statement`] from the statement dispatch;
//! - [`at_subquery`] and [`subquery`] from expressions: `(SELECT …)`,
//!   `IN (SELECT …)`, `exists(SELECT …)`, `x = ANY (SELECT …)`;
//! - [`query`] wherever a query follows without parentheses of its own, such
//!   as `CREATE MATERIALIZED VIEW … AS SELECT …`.

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::names;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Whether the statement at the current token is a query: it starts with
/// `SELECT`, `WITH` or `(`.
pub(crate) fn at_statement(p: &Parser<'_>) -> bool {
    p.at_kw(K::Select) || p.at_kw(K::With) || p.at(T::OpeningRoundBracket)
}

/// A query statement, up to the end of the statement.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the signature of the seam, for the grammar that replaces this stub"
)]
pub(crate) fn statement(p: &mut Parser<'_>) -> Option<()> {
    fallback::verbatim_statement(p);
    Some(())
}

/// A query, up to the end of the statement or a closing bracket that does
/// not belong to it.
pub(crate) fn query(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    fallback::opaque_until(p, &fallback::never)
}

/// Whether the `n`th token opens a subquery: `(` followed by `SELECT`,
/// `WITH` or `EXPLAIN`, or by `FROM` and a table (`(FROM t)`, which
/// `(from + 1)`, over a column named `from`, is not).
pub(crate) fn at_subquery(p: &Parser<'_>, n: usize) -> bool {
    p.nth_at(n, T::OpeningRoundBracket)
        && match p.nth_keyword(n + 1) {
            Some(K::Select | K::With | K::Explain) => true,
            Some(K::From) => names::at_name(p, n + 2) || p.nth_at(n + 2, T::OpeningRoundBracket),
            _ => false,
        }
}

/// A [`Subquery`](SyntaxKind::Subquery): `(`, a [`query`], `)`.
pub(crate) fn subquery(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect(T::OpeningRoundBracket)?;
    query(p)?;
    p.expect(T::ClosingRoundBracket)?;
    Some(m.complete(p, SyntaxKind::Subquery))
}
