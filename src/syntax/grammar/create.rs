//! `CREATE TABLE` and `CREATE MATERIALIZED VIEW`, as ClickHouse's
//! `ParserCreateTableQuery` and `ParserCreateViewQuery` read them
//! (`src/Parsers/ParserCreateQuery.cpp`). Materialized views are not parsed
//! yet: they are kept verbatim.
//!
//! A statement is a [`CreateHeader`](SyntaxKind::CreateHeader) (keywords,
//! name, `UUID`, `ON CLUSTER`), the
//! [`TableElementList`](SyntaxKind::TableElementList) if there is one (see
//! [`elements`]), and then clauses, one node each in the order written: the
//! storage clauses (see [`storage`]) and `COMMENT`. ClickHouse fixes their
//! order; here they may come in any order and repeat, so that each branch of
//! a Jinja `{% if %}` can hold its own clause, and a clause that does not
//! parse is kept verbatim on its own.
//!
//! A projection's query goes through
//! [`select::subquery`](crate::syntax::grammar::select::subquery).

mod elements;
mod storage;

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::names;
use crate::syntax::parser::CompletedMarker;
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
pub(crate) fn statement(p: &mut Parser<'_>) -> Option<()> {
    let mut n = 1;
    while matches!(p.nth_keyword(n), Some(K::Or | K::Replace | K::Temporary)) {
        n += 1;
    }
    if p.nth_at_kw(n, K::Materialized) {
        fallback::verbatim_statement(p);
        Some(())
    } else {
        table(p)
    }
}

/// [`CreateTable`](SyntaxKind::CreateTable).
fn table(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    header(p, &[K::Table])?;
    if p.at(T::OpeningRoundBracket) {
        elements::element_list(p)?;
    }
    clauses(p);
    m.complete(p, SyntaxKind::CreateTable);
    Some(())
}

/// `CREATE [OR REPLACE] [TEMPORARY] <object> [IF NOT EXISTS] name [UUID
/// '…'] [TO INNER UUID '…'] [ON CLUSTER c]`, where `object` is the keywords
/// of the object kind.
fn header(p: &mut Parser<'_>, object: &[K]) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect_kw(K::Create)?;
    p.eat_kws(&[K::Or, K::Replace]);
    p.eat_kw(K::Temporary);
    for &keyword in object {
        p.expect_kw(keyword)?;
    }
    common::if_not_exists(p);
    names::compound_name(p)?;
    if p.at_kw(K::Uuid) && p.nth_at(1, T::StringLiteral) {
        uuid(p, 1)?;
    }
    if p.at_kws(&[K::To, K::Inner, K::Uuid]) && p.nth_at(3, T::StringLiteral) {
        uuid(p, 3)?;
    }
    if p.at_kws(&[K::On, K::Cluster]) {
        common::on_cluster(p)?;
    }
    Some(m.complete(p, SyntaxKind::CreateHeader))
}

/// A [`TableUuid`](SyntaxKind::TableUuid): `keywords` keywords and a string.
fn uuid(p: &mut Parser<'_>, keywords: usize) -> Option<CompletedMarker> {
    let m = p.start();
    for _ in 0..keywords {
        p.bump_keyword();
    }
    exprs::literal(p)?;
    Some(m.complete(p, SyntaxKind::TableUuid))
}

/// Clauses up to the end of the statement, each with the clause fallback.
fn clauses(p: &mut Parser<'_>) {
    while !p.at_end() {
        fallback::clause(p, &at_clause, clause);
    }
}

/// Whether the current token starts one of the clauses of `CREATE TABLE`.
///
/// This is also where lists inside clauses (`TTL`, `SETTINGS`) end, so a
/// word that starts a clause only does so when what follows cannot continue
/// an expression: `ttl + 1` and `engine = 1` are not clauses.
fn at_clause(p: &Parser<'_>) -> bool {
    storage::at_storage_clause(p) || (p.at_kw(K::Comment) && p.nth_at(1, T::StringLiteral))
}

/// Whether the `n`th token can start an operand, and is not an operator:
/// what follows `TTL` when it starts a clause.
fn at_operand(p: &Parser<'_>, n: usize) -> bool {
    matches!(
        p.nth(n),
        Some(
            T::BareWord
                | T::QuotedIdentifier
                | T::Number
                | T::StringLiteral
                | T::HereDoc
                | T::OpeningRoundBracket
                | T::OpeningSquareBracket
                | T::OpeningCurlyBrace
                | T::JinjaExpr
                | T::Verbatim
        )
    )
}

/// The clause at the current token.
fn clause(p: &mut Parser<'_>) -> Option<()> {
    if storage::at_storage_clause(p) {
        return storage::storage_clause(p, &at_clause);
    }
    match p.nth_keyword(0) {
        Some(K::Comment) => comment(p),
        _ => p.expected("a clause"),
    }
}

/// `COMMENT '…'` as a [`CommentClause`](SyntaxKind::CommentClause).
fn comment(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    string(p)?;
    m.complete(p, SyntaxKind::CommentClause);
    Some(())
}

/// A string literal.
fn string(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !p.at(T::StringLiteral) {
        return p.expected("a string");
    }
    exprs::literal(p)
}

/// A name, or a name with arguments as a
/// [`FunctionCall`](SyntaxKind::FunctionCall) of the [`Name`](SyntaxKind::Name)
/// and an [`ArgList`](SyntaxKind::ArgList), or two for parameters
/// (`ParserIdentifierWithOptionalParameters`): an engine, a codec, an index
/// type. The name is never a keyword slot and never one of the functions
/// with syntax of their own, so it keeps its case.
fn name_with_args(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let name = names::name(p)?;
    if !p.at(T::OpeningRoundBracket) {
        return Some(name);
    }
    let m = name.precede(p);
    exprs::arg_list(p)?;
    if p.at(T::OpeningRoundBracket) {
        exprs::arg_list(p)?;
    }
    Some(m.complete(p, SyntaxKind::FunctionCall))
}

#[cfg(test)]
mod tests;
