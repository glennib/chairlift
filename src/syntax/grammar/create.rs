//! `CREATE TABLE` and `CREATE MATERIALIZED VIEW`.
//!
//! Not parsed yet: these statements are kept verbatim. [`at_statement`] and
//! [`statement`] are the seams the statement dispatch calls.

use crate::keyword::Keyword as K;
use crate::syntax::grammar::fallback;
use crate::syntax::parser::Parser;

/// Whether the statement at the current token is `CREATE [OR REPLACE]
/// [TEMPORARY] TABLE` or `CREATE MATERIALIZED VIEW`.
pub(crate) fn at_statement(p: &Parser<'_>) -> bool {
    if !p.at_kw(K::Create) {
        return false;
    }
    let mut n = 1;
    if p.nth_at_kw(n, K::Or) && p.nth_at_kw(n + 1, K::Replace) {
        n += 2;
    }
    if p.nth_at_kw(n, K::Temporary) {
        n += 1;
    }
    p.nth_at_kw(n, K::Table) || (p.nth_at_kw(n, K::Materialized) && p.nth_at_kw(n + 1, K::View))
}

/// A `CREATE TABLE` or `CREATE MATERIALIZED VIEW` statement, up to the end
/// of the statement.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the signature of the seam, for the grammar that replaces this stub"
)]
pub(crate) fn statement(p: &mut Parser<'_>) -> Option<()> {
    fallback::verbatim_statement(p);
    Some(())
}
