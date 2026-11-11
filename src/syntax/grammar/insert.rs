//! `INSERT`, as ClickHouse's `ParserInsertQuery` reads it
//! (`src/Parsers/ParserInsertQuery.cpp`).
//!
//! A statement is an [`InsertQuery`](SyntaxKind::InsertQuery): any `WITH`
//! clause, the [`InsertInto`](SyntaxKind::InsertInto) header with the
//! target and its columns, then, in the order ClickHouse fixes, `FROM
//! INFILE`, `SETTINGS`, and either `VALUES` or `FORMAT` with the data, or a
//! query with any `FORMAT` after it.
//!
//! The data is one [`InsertData`](SyntaxKind::InsertData) token, which the
//! splitter cuts where `clickhouse-client` ends it (see
//! [`split`](mod@crate::split)); the grammar only takes it where it stands.
//! A statement whose data the splitter and the grammar place differently
//! does not parse, and stays verbatim.
//!
//! # Keyword slots
//!
//! The words ClickHouse matches as keywords at a place are keywords there
//! whatever follows, as in `ParserInsertQuery`: `TABLE` and `FUNCTION`
//! right after `INTO` (`INSERT INTO table table` names a table `table`),
//! `FROM INFILE`, `SETTINGS`, and `VALUES` and `FORMAT` after the columns
//! and settings. A setting's name after `SETTINGS` is a name, so in `INSERT
//! INTO t SETTINGS format = 1 FORMAT TSV` only the second `format` is a
//! keyword.

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::exprs::AliasMode;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::names;
use crate::syntax::grammar::select;
use crate::syntax::grammar::select::Level;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Whether the statement at the current token is `INSERT INTO`.
pub(crate) fn at_statement(p: &Parser<'_>) -> bool {
    p.at_kws(&[K::Insert, K::Into])
}

/// An `INSERT` statement, with or without a `WITH` clause before it, up to
/// the end of the statement.
///
/// A statement with ClickHouse's pipe syntax (`|>`) after `INSERT … FROM t`
/// fails as a whole, as a query statement with it does (see
/// [`select::statement`]).
pub(crate) fn statement(p: &mut Parser<'_>) -> Option<()> {
    if (0..)
        .map_while(|n| p.nth(n))
        .any(|kind| kind == T::PipeOperator)
    {
        return p.expected("an INSERT without `|>`");
    }
    let m = p.start();
    if p.at_kw(K::With) {
        with_clause(p)?;
    }
    into(p)?;
    let infile = p.at_kws(&[K::From, K::Infile]);
    if infile {
        infile_clause(p)?;
    }
    if p.at_kw(K::Settings) {
        common::settings_clause(p, &at_body)?;
    }
    if !infile && p.at_kw(K::Values) {
        values(p);
    } else if p.at_kw(K::Format) {
        format(p)?;
    } else if select::at_statement(p) {
        select::query(p, Level::Nested)?;
        if p.at_kw(K::Format) {
            format(p)?;
        }
    } else if !infile {
        return p.expected("VALUES, FORMAT or a query");
    }
    m.complete(p, SyntaxKind::InsertQuery);
    Some(())
}

/// Whether what follows the settings starts here: `VALUES`, `FORMAT` or a
/// query.
fn at_body(p: &Parser<'_>) -> bool {
    p.at_kw(K::Values) || p.at_kw(K::Format) || select::at_statement(p)
}

/// `WITH` and its elements before `INSERT`, as a
/// [`WithClause`](SyntaxKind::WithClause) with a
/// [`WithList`](SyntaxKind::WithList), as a query's: CTEs and expressions
/// with an `AS` alias (`ParserWithElement`). There is no `RECURSIVE` here.
fn with_clause(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    let list = p.start();
    let shape = fallback::comma_separated(p, &at_statement, &mut |p| {
        if select::at_common_table_expr(p) {
            select::common_table_expr(p)
        } else {
            exprs::expr_with_alias(p, AliasMode::As).map(drop)
        }
    });
    if shape.items == 0 {
        list.abandon(p);
        return p.expected("a WITH element");
    }
    list.complete(p, SyntaxKind::WithList);
    m.complete(p, SyntaxKind::WithClause);
    if !at_statement(p) {
        return p.expected("INSERT INTO");
    }
    Some(())
}

/// [`InsertInto`](SyntaxKind::InsertInto): `INSERT INTO [TABLE]`, the
/// target, and any column list. A `(` that opens a query is not a column
/// list: `INSERT INTO t (SELECT 1)` inserts the query, as ClickHouse
/// decides by failing to read the brackets as columns.
fn into(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    if !p.eat_kws(&[K::Insert, K::Into]) {
        m.abandon(p);
        return p.expected("INSERT INTO");
    }
    p.eat_kw(K::Table);
    if p.eat_kw(K::Function) {
        select::table_function(p)?;
        if p.at_kws(&[K::Partition, K::By]) {
            partition_by(p)?;
        }
    } else {
        table_name(p)?;
    }
    if p.at(T::OpeningRoundBracket) && !select::at_subquery(p, 0) {
        columns(p)?;
    }
    Some(m.complete(p, SyntaxKind::InsertInto))
}

/// `name` or `db.name` (`ParserIdentifier` twice, with a `.` between): a
/// [`Name`](SyntaxKind::Name) or a
/// [`QualifiedName`](SyntaxKind::QualifiedName) of two.
pub(crate) fn table_name(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let first = names::name(p)?;
    if !(p.at(T::Dot) && names::at_name(p, 1)) {
        return Some(first);
    }
    let m = first.precede(p);
    p.bump();
    names::name(p)?;
    Some(m.complete(p, SyntaxKind::QualifiedName))
}

/// `PARTITION BY expr` after a table function, as a
/// [`PartitionByClause`](SyntaxKind::PartitionByClause); the expression may
/// have an `AS` alias, as ClickHouse allows.
fn partition_by(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.bump_keyword();
    exprs::expr_with_alias(p, AliasMode::As)?;
    m.complete(p, SyntaxKind::PartitionByClause);
    Some(())
}

/// The column list, an [`InsertColumns`](SyntaxKind::InsertColumns)
/// (`ParserInsertElement`): names, `*` and `t.*` with any transformers, and
/// `COLUMNS(…)` matchers; a trailing comma is allowed.
fn columns(p: &mut Parser<'_>) -> Option<()> {
    fallback::delimited(
        p,
        SyntaxKind::InsertColumns,
        T::OpeningRoundBracket,
        T::ClosingRoundBracket,
        &mut column,
    )
    .map(drop)
}

/// One element of the column list.
fn column(p: &mut Parser<'_>) -> Option<()> {
    let matcher = if p.at(T::Asterisk) || (p.at_kw(K::Columns) && exprs::at_columns_matcher(p, 0)) {
        exprs::expr(p)?
    } else if at_qualified_matcher(p) {
        // A name, whatever it spells (`ParserQualifiedAsterisk`): `true.*`
        // takes the columns of a table called `true`.
        let qualifier = names::compound_name_with(p, true)?;
        exprs::dot(p, qualifier)?
    } else {
        return names::compound_name(p).map(drop);
    };
    if matches!(
        matcher.kind(),
        SyntaxKind::AsteriskExpr | SyntaxKind::ColumnsMatcher
    ) {
        Some(())
    } else {
        p.expected("a column, `*` or `COLUMNS(…)`")
    }
}

/// Whether dotted names and then `.*` or `.COLUMNS(…)` start here.
fn at_qualified_matcher(p: &Parser<'_>) -> bool {
    let mut n = 0;
    while let Some(name) = names::name_tokens(p, n) {
        if !p.nth_at(n + name, T::Dot) {
            return false;
        }
        n += name + 1;
        if p.nth_at(n, T::Asterisk)
            || (p.nth_text(n) == "COLUMNS" && exprs::at_columns_matcher(p, n))
        {
            return true;
        }
    }
    false
}

/// `FROM INFILE 'file' [COMPRESSION 'method']` as an
/// [`InfileClause`](SyntaxKind::InfileClause).
fn infile_clause(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.bump_keyword();
    exprs::string(p)?;
    if p.eat_kw(K::Compression) {
        exprs::string(p)?;
    }
    m.complete(p, SyntaxKind::InfileClause);
    Some(())
}

/// `VALUES` and the rows, as an [`InsertValues`](SyntaxKind::InsertValues).
fn values(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump_keyword();
    p.eat(T::InsertData);
    m.complete(p, SyntaxKind::InsertValues);
}

/// `FORMAT name` and the data, if any follows, as an
/// [`InsertFormat`](SyntaxKind::InsertFormat). The name keeps its case.
fn format(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    names::name(p)?;
    p.eat(T::InsertData);
    m.complete(p, SyntaxKind::InsertFormat);
    Some(())
}

#[cfg(test)]
mod tests;
