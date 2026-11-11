//! `CREATE TABLE` and `CREATE MATERIALIZED VIEW`, as ClickHouse's
//! `ParserCreateTableQuery` and `ParserCreateViewQuery` read them
//! (`src/Parsers/ParserCreateQuery.cpp`).
//!
//! A statement is a [`CreateHeader`](SyntaxKind::CreateHeader) (keywords,
//! name, `UUID`, `ON CLUSTER`), the
//! [`TableElementList`](SyntaxKind::TableElementList) if there is one (see
//! [`elements`]), and then clauses, one node each in the order written: the
//! storage clauses (see [`storage`]), `COMMENT`, `EMPTY` or `CLONE`, `SQL
//! SECURITY`, `AS …` and `FORMAT`, and for a view `REFRESH`, `TO` and
//! `POPULATE`. ClickHouse fixes their order; here they may come in any order
//! and repeat, so that each branch of a Jinja `{% if %}` can hold its own
//! clause, and a clause that does not parse is kept verbatim on its own.
//!
//! The query after `AS`, and a projection's query, go through
//! [`select::query`] and [`select::subquery`].

pub(crate) mod elements;
pub(crate) mod storage;

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::names;
use crate::syntax::grammar::select;
use crate::syntax::grammar::select::Level;
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
    let mut object = Object::Table;
    while let Some(keyword @ (K::Or | K::Replace | K::Temporary)) = p.nth_keyword(n) {
        if keyword == K::Temporary {
            object = Object::TemporaryTable;
        }
        n += 1;
    }
    if p.nth_at_kw(n, K::Materialized) {
        materialized_view(p)
    } else {
        table(p, object)
    }
}

/// What a statement creates, as far as its `AS` cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Object {
    Table,
    TemporaryTable,
    View,
}

/// What a statement holds before a clause, which decides what its `AS`
/// takes (see [`as_clause`]).
#[derive(Clone, Copy, Debug)]
struct Context {
    object: Object,
    /// The statement has a column list.
    columns: bool,
    /// A storage clause comes before.
    storage: bool,
}

/// [`CreateTable`](SyntaxKind::CreateTable).
fn table(p: &mut Parser<'_>, object: Object) -> Option<()> {
    let m = p.start();
    header(p, &[K::Table])?;
    let columns = p.at(T::OpeningRoundBracket);
    if columns {
        elements::element_list(p)?;
    }
    clauses(
        p,
        Context {
            object,
            columns,
            storage: false,
        },
    );
    m.complete(p, SyntaxKind::CreateTable);
    Some(())
}

/// [`CreateMaterializedView`](SyntaxKind::CreateMaterializedView). The
/// column list follows `REFRESH` and `TO`, as in ClickHouse.
fn materialized_view(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    header(p, &[K::Materialized, K::View])?;
    let context = Context {
        object: Object::View,
        columns: false,
        storage: false,
    };
    let before_columns = |p: &Parser<'_>| at_clause(p) || p.at(T::OpeningRoundBracket);
    while p.at_kw(K::Refresh) || at_to(p) {
        fallback::clause(p, &before_columns, |p| clause(p, context));
    }
    if p.at(T::OpeningRoundBracket) {
        elements::element_list(p)?;
    }
    clauses(p, context);
    m.complete(p, SyntaxKind::CreateMaterializedView);
    Some(())
}

/// `CREATE [OR REPLACE] [TEMPORARY] <object> [IF NOT EXISTS] name [UUID
/// '…'] [TO INNER UUID '…'] [ON CLUSTER c]`, where `object` is the keywords
/// of the object kind.
pub(crate) fn header(p: &mut Parser<'_>, object: &[K]) -> Option<CompletedMarker> {
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
fn clauses(p: &mut Parser<'_>, mut context: Context) {
    while !p.at_end() {
        let storage = storage::at_storage_clause(p);
        fallback::clause(p, &at_clause, |p| clause(p, context));
        context.storage |= storage;
    }
}

/// Whether the current token starts one of the clauses of `CREATE TABLE`
/// or `CREATE MATERIALIZED VIEW`.
///
/// This is also where an item of a list inside a clause (`TTL`,
/// `SETTINGS`) may end, so a word that starts a clause only does so when
/// what follows cannot continue an expression: `ttl + 1` and `engine = 1`
/// are not clauses, nor is `empty(s)`. An item after a comma is read
/// whatever word starts it (see [`fallback::comma_separated`]).
fn at_clause(p: &Parser<'_>) -> bool {
    if storage::at_storage_clause(p) || storage::at_view_target(p).is_some() {
        return true;
    }
    match p.nth_keyword(0) {
        Some(K::Definer) => p.nth_at(1, T::Equals) || names::at_name(p, 1),
        Some(K::As) => at_operand(p, 1),
        Some(K::Comment) => p.nth_at(1, T::StringLiteral),
        Some(K::Populate | K::Empty | K::Clone) => p.nth(1).is_none() || at_clause_word(p, 1),
        Some(K::Sql) => p.nth_at_kw(1, K::Security),
        Some(K::Refresh) => matches!(p.nth_keyword(1), Some(K::Every | K::After | K::Depends)),
        Some(K::To) => at_to(p),
        Some(K::Format) => common::at_format(p),
        Some(K::Into) => common::at_into_outfile(p),
        _ => false,
    }
}

/// Whether the `n`th token is a word that can start a clause after
/// `POPULATE`, `EMPTY` or `CLONE`.
fn at_clause_word(p: &Parser<'_>, n: usize) -> bool {
    matches!(
        p.nth_keyword(n),
        Some(K::As | K::Comment | K::Definer | K::Sql | K::Empty | K::Clone | K::Populate)
    )
}

/// Whether the `n`th token can start an operand, and is not an operator:
/// what follows `TTL` or `AS` when they start a clause.
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

/// Whether the current token starts a view's `TO [db.]table` or `TO INNER
/// UUID '…'`, which `TO DISK '…'` and `TO VOLUME IF EXISTS '…'` of a TTL
/// rule do not; `TO disk` names a table.
fn at_to(p: &Parser<'_>) -> bool {
    p.at_kw(K::To)
        && names::at_name(p, 1)
        && !(matches!(p.nth_keyword(1), Some(K::Disk | K::Volume))
            && (p.nth_at(2, T::StringLiteral) || p.nth_at_kw(2, K::If)))
}

/// The clause at the current token, after what `context` says.
fn clause(p: &mut Parser<'_>, context: Context) -> Option<()> {
    if storage::at_storage_clause(p) {
        return storage::storage_clause(p, &at_clause);
    }
    if storage::at_view_target(p).is_some() {
        return storage::view_target(p, &at_clause);
    }
    match p.nth_keyword(0) {
        Some(K::Comment) => comment(p),
        Some(K::Populate | K::Empty | K::Clone) => {
            let m = p.start();
            p.bump_keyword();
            m.complete(p, SyntaxKind::PopulateClause);
            Some(())
        }
        Some(K::Definer | K::Sql) => sql_security(p),
        Some(K::As) => as_clause(p, context),
        Some(K::Refresh) => refresh(p),
        Some(K::To) => to(p),
        Some(K::Format) => common::format_clause(p).map(drop),
        Some(K::Into) => common::into_outfile_clause(p).map(drop),
        _ => p.expected("a clause"),
    }
}

/// `COMMENT '…'` as a [`CommentClause`](SyntaxKind::CommentClause).
pub(crate) fn comment(p: &mut Parser<'_>) -> Option<()> {
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

/// `AS` and a query, a table function or a table, as an
/// [`AsClause`](SyntaxKind::AsClause), as `ParserCreateTableQuery` and
/// `ParserCreateViewQuery` decide: a view takes a query; a table with
/// columns a query if it has a storage clause before or is temporary, and a
/// table function otherwise, so in `CREATE TABLE t (a UInt8) AS select('x')`
/// the function is named `select`; a table without columns a query if one
/// parses, then a table function unless a storage clause comes before, then
/// a table's name.
fn as_clause(p: &mut Parser<'_>, context: Context) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    let has_storage = context.storage || context.object == Object::TemporaryTable;
    if context.object == Object::View || (context.columns && has_storage) {
        select::query(p, Level::Nested)?;
    } else if context.columns {
        select::table_function(p)?;
    } else if !as_query(p)? {
        if !context.storage && names::at_name(p, 0) && p.nth_at(1, T::OpeningRoundBracket) {
            select::table_function(p)?;
        } else {
            names::compound_name(p)?;
        }
    }
    m.complete(p, SyntaxKind::AsClause);
    Some(())
}

/// The query after `AS` of a table without columns, which ClickHouse tries
/// before a table function or a name: whether one parsed, or `None` if a
/// query that must be one failed. `select` before `.` or the end names a
/// table, and a `WITH` clause is a query only if `SELECT` or `FROM` follows
/// it, so `AS with(1)` calls a table function and `AS with.with` names a
/// table.
fn as_query(p: &mut Parser<'_>) -> Option<bool> {
    if !select::at_statement(p) || (p.at_kw(K::Select) && matches!(p.nth(1), None | Some(T::Dot))) {
        return Some(false);
    }
    if p.at_kw(K::With) {
        return Some(p.speculate(|p| select::query(p, Level::Nested)).is_some());
    }
    select::query(p, Level::Nested).map(|_| true)
}

/// `DEFINER = user` and `SQL SECURITY DEFINER|INVOKER|NONE`, in either order,
/// as a [`SqlSecurityClause`](SyntaxKind::SqlSecurityClause)
/// (`ParserSQLSecurity`).
pub(crate) fn sql_security(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    let mut definer = false;
    let mut security = false;
    loop {
        if !definer && p.at_kw(K::Definer) {
            definer = true;
            p.bump_keyword();
            p.eat(T::Equals);
            if !p.eat_kw(K::CurrentUser) {
                user_name(p)?;
            }
        } else if !security && p.at_kws(&[K::Sql, K::Security]) {
            security = true;
            p.bump_keyword();
            p.bump_keyword();
            if !matches!(p.nth_keyword(0), Some(K::Definer | K::Invoker | K::None)) {
                return p.expected("DEFINER, INVOKER or NONE");
            }
            p.bump_keyword();
        } else {
            break;
        }
    }
    if !definer && !security {
        return p.expected("DEFINER or SQL SECURITY");
    }
    m.complete(p, SyntaxKind::SqlSecurityClause);
    Some(())
}

/// A user name: `'name'`, or `name` with an optional `@host`
/// (`parseUserNameWithHost`).
fn user_name(p: &mut Parser<'_>) -> Option<()> {
    if p.at(T::StringLiteral) {
        return exprs::literal(p).map(drop);
    }
    names::name(p)?;
    if p.eat(T::At) {
        if p.at(T::StringLiteral) {
            exprs::literal(p)?;
        } else {
            names::name(p)?;
        }
    }
    Some(())
}

/// `TO [db.]table` or `TO INNER UUID '…'` of a view, as a
/// [`ToClause`](SyntaxKind::ToClause).
fn to(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    if p.at_kws(&[K::Inner, K::Uuid]) && p.nth_at(2, T::StringLiteral) {
        p.bump_keyword();
        p.bump_keyword();
        exprs::literal(p)?;
    } else {
        names::compound_name(p)?;
    }
    m.complete(p, SyntaxKind::ToClause);
    Some(())
}

/// `REFRESH …` of a materialized view, as a
/// [`RefreshClause`](SyntaxKind::RefreshClause) (`ParserRefreshStrategy`).
pub(crate) fn refresh(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    let every = p.at_kw(K::Every);
    let scheduled = every || p.at_kw(K::After);
    if scheduled {
        p.bump_keyword();
        time_interval(p)?;
        if every && p.eat_kw(K::Offset) {
            time_interval(p)?;
        }
    }
    if p.eat_kws(&[K::Randomize, K::For]) {
        time_interval(p)?;
    }
    if p.at_kws(&[K::Depends, K::On]) {
        depends_on(p)?;
    } else if !scheduled {
        return p.expected("EVERY, AFTER or DEPENDS ON");
    }
    if common::at_settings(p) {
        common::settings_clause(p, &|p: &Parser<'_>| at_clause(p) || p.at_kw(K::Append))?;
    }
    if p.eat_kw(K::Append) {
        p.eat_kw(K::Incremental);
    }
    m.complete(p, SyntaxKind::RefreshClause);
    Some(())
}

/// `n UNIT [n UNIT …]` as a [`TimeInterval`](SyntaxKind::TimeInterval)
/// (`ParserTimeInterval`).
fn time_interval(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let mut parts = 0;
    while p.at(T::Number) && exprs::at_interval_unit(p, 1) {
        exprs::literal(p)?;
        let unit = p.start();
        p.bump_keyword();
        unit.complete(p, SyntaxKind::IntervalUnit);
        parts += 1;
    }
    if parts == 0 {
        m.abandon(p);
        return p.expected("an interval");
    }
    Some(m.complete(p, SyntaxKind::TimeInterval))
}

/// `DEPENDS ON a, db.b` as a [`DependsOn`](SyntaxKind::DependsOn).
fn depends_on(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.bump_keyword();
    p.bump_keyword();
    names::compound_name(p)?;
    while p.at(T::Comma) && names::at_name(p, 1) {
        p.bump();
        names::compound_name(p)?;
    }
    Some(m.complete(p, SyntaxKind::DependsOn))
}

/// A name, or a name with arguments as a
/// [`FunctionCall`](SyntaxKind::FunctionCall) of the [`Name`](SyntaxKind::Name)
/// and an [`ArgList`](SyntaxKind::ArgList), or two for parameters
/// (`ParserIdentifierWithOptionalParameters`): an engine, a codec, an index
/// type. The name is never a keyword slot and never one of the functions
/// with syntax of their own, so it keeps its case.
pub(crate) fn name_with_args(p: &mut Parser<'_>) -> Option<CompletedMarker> {
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
