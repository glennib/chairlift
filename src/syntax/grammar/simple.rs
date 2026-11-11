//! Statements of a line or two: `DROP`, `DETACH`, `TRUNCATE`, `UNDROP`, the
//! short `ATTACH`, `RENAME`, `EXCHANGE`, `OPTIMIZE`, `SET` and `USE`, as
//! ClickHouse's `ParserDropQuery`, `ParserDropFunctionQuery` and its
//! relatives for named collections, indexes, workloads, resources and
//! handlers, `ParserUndropQuery`, `ParserCreateQuery`, `ParserRenameQuery`,
//! `ParserOptimizeQuery`, `ParserSetQuery` and `ParserUseQuery` read them.
//!
//! Each is one node, with its keywords as tokens and its names, lists and
//! clauses as nodes in the order written, which is the order ClickHouse
//! fixes. The statements that `ParserQueryWithOutput` reads (`DROP`,
//! `DETACH`, `TRUNCATE` and `UNDROP` of a table, view, dictionary or
//! database; `RENAME`, `EXCHANGE` and `OPTIMIZE`) end with any output
//! clauses: `INTO OUTFILE`, `FORMAT` and `SETTINGS`.
//!
//! An `ATTACH` with a definition after the name (`ATTACH TABLE t (a UInt8)
//! ENGINE = …`, `ATTACH DATABASE d ENGINE = …`) has its head parsed and the
//! definition kept as written, [`Opaque`](SyntaxKind::Opaque) by design.
//!
//! Left to other grammars, and verbatim here: `DROP`, `ATTACH` and `SET` of
//! access entities (`DROP USER`, `SET ROLE`, `SET DEFAULT ROLE`), and `SET
//! TRANSACTION SNAPSHOT`.
//!
//! # Keyword slots
//!
//! A word ClickHouse matches as a keyword at a place is one there whatever
//! follows: `IF EXISTS` and `IF EMPTY` before a name, `ON` after one (which
//! `CLUSTER` must follow), `TABLE` after `TRUNCATE`, `ID` and `ALL` after
//! `PARTITION`. Names are names whatever they spell: `DROP TABLE sync SYNC`,
//! `RENAME TABLE a TO to`, `OPTIMIZE TABLE final FINAL`, `USE system`, `SET
//! format = 'x'`. `USE DATABASE` is the keyword only before a name, so
//! `USE database` uses a database called `database`.

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::fallback::Stop;
use crate::syntax::grammar::insert::table_name;
use crate::syntax::grammar::names;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Whether the statement at the current token is one of this module's.
pub(crate) fn at_statement(p: &Parser<'_>) -> bool {
    match p.nth_keyword(0) {
        Some(K::Drop) => {
            matches!(
                p.nth_keyword(1),
                Some(
                    K::Table
                        | K::View
                        | K::Dictionary
                        | K::Database
                        | K::Temporary
                        | K::Function
                        | K::Index
                        | K::Workload
                        | K::Resource
                        | K::Handler
                )
            ) || (p.nth_at_kw(1, K::Named) && p.nth_at_kw(2, K::Collection))
        }
        Some(K::Detach) => matches!(
            p.nth_keyword(1),
            Some(K::Table | K::View | K::Dictionary | K::Database | K::Temporary)
        ),
        Some(K::Truncate | K::Rename | K::Use) => true,
        Some(K::Undrop | K::Optimize) => p.nth_at_kw(1, K::Table),
        Some(K::Exchange) => matches!(p.nth_keyword(1), Some(K::Tables | K::Dictionaries)),
        Some(K::Attach) => {
            matches!(
                p.nth_keyword(1),
                Some(K::Table | K::Temporary | K::Dictionary | K::Database | K::View)
            ) || (p.nth_at_kw(1, K::Materialized) && p.nth_at_kw(2, K::View))
        }
        // `SET ROLE …` and `SET DEFAULT ROLE …` (`ParserSetRoleQuery`, tried
        // first) set roles unless `=` makes `role` a setting; `SET
        // TRANSACTION` is never a setting.
        Some(K::Set) => match p.nth_keyword(1) {
            Some(K::Transaction) => false,
            Some(K::Role) => p.nth_at(2, T::Equals),
            Some(K::Default) => !p.nth_at_kw(2, K::Role),
            _ => true,
        },
        _ => false,
    }
}

/// A statement of this module's, up to the end of the statement.
pub(crate) fn statement(p: &mut Parser<'_>) -> Option<()> {
    match p.nth_keyword(0) {
        Some(K::Drop | K::Detach | K::Truncate | K::Undrop) => drop_query(p),
        Some(K::Attach) => attach(p),
        Some(K::Rename | K::Exchange) => rename(p),
        Some(K::Optimize) => optimize(p),
        Some(K::Set) => set(p),
        Some(K::Use) => use_database(p),
        _ => p.expected("a statement"),
    }
}

/// A [`DropQuery`](SyntaxKind::DropQuery): `DROP`, `DETACH`, `TRUNCATE` or
/// `UNDROP`.
fn drop_query(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    let verb = p.nth_keyword(0);
    p.bump_keyword();
    let output = match (verb, p.nth_keyword(0)) {
        (Some(K::Undrop), _) => {
            p.expect_kw(K::Table)?;
            table_name(p)?;
            if p.at_kw(K::Uuid) {
                uuid(p)?;
            }
            true
        }
        (Some(K::Drop), Some(K::Function | K::Workload | K::Resource | K::Handler)) => {
            p.bump_keyword();
            common::if_exists(p);
            names::name(p)?;
            false
        }
        (Some(K::Drop), Some(K::Named)) => {
            p.bump_keyword();
            p.expect_kw(K::Collection)?;
            common::if_exists(p);
            names::name(p)?;
            false
        }
        (Some(K::Drop), Some(K::Index)) => {
            p.bump_keyword();
            common::if_exists(p);
            names::name(p)?;
            p.expect_kw(K::On)?;
            table_name(p)?;
            false
        }
        _ => {
            drop_object(p, verb)?;
            true
        }
    };
    if p.at_kw(K::On) {
        common::on_cluster(p)?;
    }
    if output {
        if verb == Some(K::Detach) {
            p.eat_kw(K::Permanently);
        }
        if verb != Some(K::Undrop) && !p.eat_kws(&[K::No, K::Delay]) {
            p.eat_kw(K::Sync);
        }
        output_clauses(p);
    }
    m.complete(p, SyntaxKind::DropQuery);
    Some(())
}

/// What `DROP`, `DETACH` and `TRUNCATE` take (`parseDropQuery`): `DATABASE`
/// and a name; for `TRUNCATE`, `[ALL] TABLES FROM` a database with any
/// `[NOT] LIKE|ILIKE 'pattern'`; otherwise `[TEMPORARY] TABLE|VIEW|DICTIONARY`
/// (`TABLE` may be left out after `TRUNCATE`) and the names, which only
/// `DROP` may list.
fn drop_object(p: &mut Parser<'_>, verb: Option<K>) -> Option<()> {
    let truncate = verb == Some(K::Truncate);
    if p.eat_kw(K::Database) {
        if_exists_or_empty(p);
        return names::name(p).map(drop);
    }
    if truncate && (p.at_kw(K::Tables) || p.at_kws(&[K::All, K::Tables])) {
        p.eat_kw(K::All);
        p.bump_keyword();
        p.expect_kw(K::From)?;
        common::if_exists(p);
        names::name(p)?;
        if p.at_kw(K::Not) && matches!(p.nth_keyword(1), Some(K::Like | K::Ilike)) {
            p.bump_keyword();
        }
        if p.at_kw(K::Like) || p.at_kw(K::Ilike) {
            p.bump_keyword();
            exprs::string(p)?;
        }
        return Some(());
    }
    p.eat_kw(K::Temporary);
    if !(p.eat_kw(K::View) || p.eat_kw(K::Dictionary) || p.eat_kw(K::Table) || truncate) {
        return p.expected("TABLE, VIEW or DICTIONARY");
    }
    if_exists_or_empty(p);
    if verb == Some(K::Drop) {
        name_list(p, &at_drop_tail)?;
    } else {
        names::compound_name(p)?;
    }
    Some(())
}

/// Any `IF EXISTS`, then any `IF EMPTY`.
fn if_exists_or_empty(p: &mut Parser<'_>) {
    common::if_exists(p);
    p.eat_kws(&[K::If, K::Empty]);
}

/// Whether what follows the names of `DROP` starts here.
fn at_drop_tail(p: &Parser<'_>) -> bool {
    matches!(p.nth_keyword(0), Some(K::On | K::Permanently | K::Sync))
        || p.at_kws(&[K::No, K::Delay])
        || at_output_clause(p)
}

/// Dotted names separated by commas, as a
/// [`NameList`](SyntaxKind::NameList), with items that end at `ends`.
fn name_list(p: &mut Parser<'_>, ends: Stop<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let shape = fallback::comma_separated(p, ends, &mut |p| names::compound_name(p).map(drop));
    if shape.items == 0 {
        m.abandon(p);
        return p.expected("a name");
    }
    Some(m.complete(p, SyntaxKind::NameList))
}

/// `UUID '…'` as a [`TableUuid`](SyntaxKind::TableUuid).
fn uuid(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.bump_keyword();
    exprs::string(p)?;
    Some(m.complete(p, SyntaxKind::TableUuid))
}

/// An [`AttachQuery`](SyntaxKind::AttachQuery): `ATTACH`, the object and
/// its name as `ParserCreateQuery` reads them before a definition, `FROM
/// 'path'` or `AS [NOT] REPLICATED` for a table, any `ON CLUSTER`, and what
/// follows kept [`Opaque`](SyntaxKind::Opaque).
fn attach(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    let table = p.at_kw(K::Table) || p.at_kw(K::Temporary);
    let database = p.at_kw(K::Database);
    if table {
        p.eat_kw(K::Temporary);
        p.expect_kw(K::Table)?;
    } else if p.at_kw(K::Materialized) || p.at_kw(K::View) {
        p.eat_kw(K::Materialized);
        p.expect_kw(K::View)?;
    } else if !(p.eat_kw(K::Dictionary) || p.eat_kw(K::Database)) {
        return p.expected("TABLE, VIEW, DICTIONARY or DATABASE");
    }
    common::if_not_exists(p);
    if database {
        names::name(p)?;
    } else {
        table_name(p)?;
    }
    if p.at_kw(K::Uuid) && p.nth_at(1, T::StringLiteral) {
        uuid(p)?;
    }
    if table && p.at_kw(K::From) && p.nth_at(1, T::StringLiteral) {
        p.bump_keyword();
        exprs::literal(p)?;
    } else if table
        && p.at_kw(K::As)
        && (p.nth_at_kw(1, K::Replicated)
            || (p.nth_at_kw(1, K::Not) && p.nth_at_kw(2, K::Replicated)))
    {
        p.bump_keyword();
        p.eat_kw(K::Not);
        p.bump_keyword();
    }
    if p.at_kw(K::On) {
        common::on_cluster(p)?;
    }
    if !p.at_end() {
        fallback::opaque_until(p, &fallback::never)?;
    }
    m.complete(p, SyntaxKind::AttachQuery);
    Some(())
}

/// A [`RenameQuery`](SyntaxKind::RenameQuery): `RENAME DATABASE` a database,
/// or `RENAME [TABLE | DICTIONARY]` or `EXCHANGE TABLES | DICTIONARIES` and
/// a [`RenameList`](SyntaxKind::RenameList); then any `ON CLUSTER`.
fn rename(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    let exchange = p.at_kw(K::Exchange);
    let database = p.at_kws(&[K::Rename, K::Database]);
    p.bump_keyword();
    if database || exchange {
        p.bump_keyword();
    } else if !p.eat_kw(K::Table) {
        p.eat_kw(K::Dictionary);
    }
    let list = p.start();
    let shape = fallback::comma_separated(
        p,
        &|p: &Parser<'_>| p.at_kw(K::On) || at_output_clause(p),
        &mut |p| rename_pair(p, exchange, database),
    );
    if shape.items == 0 || (database && shape.items + shape.commas > 1) {
        list.abandon(p);
        return p.expected("what to rename");
    }
    list.complete(p, SyntaxKind::RenameList);
    if p.at_kw(K::On) {
        common::on_cluster(p)?;
    }
    output_clauses(p);
    m.complete(p, SyntaxKind::RenameQuery);
    Some(())
}

/// A [`RenamePair`](SyntaxKind::RenamePair): `[IF EXISTS] a TO b`, or `a AND
/// b` to exchange; a database's names have no `.`.
fn rename_pair(p: &mut Parser<'_>, exchange: bool, database: bool) -> Option<()> {
    let m = p.start();
    let name = |p: &mut Parser<'_>| {
        if database {
            names::name(p)
        } else {
            table_name(p)
        }
    };
    if !exchange {
        common::if_exists(p);
    }
    name(p)?;
    p.expect_kw(if exchange { K::And } else { K::To })?;
    name(p)?;
    m.complete(p, SyntaxKind::RenamePair);
    Some(())
}

/// An [`OptimizeQuery`](SyntaxKind::OptimizeQuery), its parts in the order
/// `ParserOptimizeQuery` reads them: `OPTIMIZE TABLE` and a name, any `ON
/// CLUSTER`, [`Partition`](SyntaxKind::Partition), `DRY RUN PARTS` and an
/// [`ExprList`](SyntaxKind::ExprList) of part names, `FINAL` or `FORCE`,
/// `DEDUPLICATE`, `CLEANUP`, `MANIFEST`, and `BY` and an `ExprList` of the
/// columns to deduplicate by.
fn optimize(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.expect_kw(K::Table)?;
    table_name(p)?;
    if p.at_kw(K::On) {
        common::on_cluster(p)?;
    }
    if p.at_kw(K::Partition) {
        partition(p)?;
    }
    if p.eat_kws(&[K::Dry, K::Run]) {
        p.expect_kw(K::Parts)?;
        let at_flag = |p: &Parser<'_>| {
            matches!(
                p.nth_keyword(0),
                Some(K::Final | K::Force | K::Deduplicate | K::Cleanup | K::Manifest)
            ) || at_output_clause(p)
        };
        list(p, &at_flag, &mut |p| exprs::string(p).map(drop))?;
    }
    if !p.eat_kw(K::Final) {
        p.eat_kw(K::Force);
    }
    let deduplicate = p.eat_kw(K::Deduplicate);
    p.eat_kw(K::Cleanup);
    p.eat_kw(K::Manifest);
    if deduplicate && p.eat_kw(K::By) {
        list(p, &at_output_clause, &mut deduplicate_column)?;
    }
    output_clauses(p);
    m.complete(p, SyntaxKind::OptimizeQuery);
    Some(())
}

/// Items separated by commas as an [`ExprList`](SyntaxKind::ExprList), with
/// items that end at `ends`. Fails if there is none.
fn list(
    p: &mut Parser<'_>,
    ends: Stop<'_>,
    item: &mut dyn FnMut(&mut Parser<'_>) -> Option<()>,
) -> Option<CompletedMarker> {
    let m = p.start();
    let shape = fallback::comma_separated(p, ends, item);
    if shape.items == 0 {
        m.abandon(p);
        return p.expected("a list item");
    }
    Some(m.complete(p, SyntaxKind::ExprList))
}

/// A column of `DEDUPLICATE BY` (`ParserOptimizeQueryColumnsSpecification`):
/// a name, or `*` or `COLUMNS(…)` with any `EXCEPT`.
fn deduplicate_column(p: &mut Parser<'_>) -> Option<()> {
    if !(p.at(T::Asterisk) || (p.at_kw(K::Columns) && exprs::at_columns_matcher(p, 0))) {
        return names::name(p).map(drop);
    }
    let matcher = exprs::expr(p)?;
    if matches!(
        matcher.kind(),
        SyntaxKind::AsteriskExpr | SyntaxKind::ColumnsMatcher
    ) {
        Some(())
    } else {
        p.expected("a column, `*` or `COLUMNS(…)`")
    }
}

/// `PARTITION` and a partition, as a [`Partition`](SyntaxKind::Partition)
/// (`ParserPartition`): `ID` and a string or a query parameter, `ALL`, or
/// an expression.
pub(crate) fn partition(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect_kw(K::Partition)?;
    if p.eat_kw(K::Id) {
        if names::at_query_parameter(p, 0) {
            exprs::query_parameter(p)?;
        } else {
            exprs::string(p)?;
        }
    } else if !p.eat_kw(K::All) {
        exprs::expr(p)?;
    }
    Some(m.complete(p, SyntaxKind::Partition))
}

/// A [`SetQuery`](SyntaxKind::SetQuery): `SET` and a
/// [`SettingsList`](SyntaxKind::SettingsList), or `SET TIME ZONE [=]` and a
/// string, number or map of strings.
fn set(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    if p.eat_kws(&[K::Time, K::Zone]) {
        p.eat(T::Equals);
        if p.at(T::OpeningCurlyBrace) {
            exprs::map_literal(p, &mut |p| exprs::string(p).map(drop))?;
        } else {
            exprs::literal(p)?;
        }
    } else {
        common::settings_list(p, &fallback::never)?;
    }
    m.complete(p, SyntaxKind::SetQuery);
    Some(())
}

/// A [`UseQuery`](SyntaxKind::UseQuery): `USE [DATABASE] name`.
fn use_database(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    if p.at_kw(K::Database) && names::at_name(p, 1) {
        p.bump_keyword();
    }
    names::name(p)?;
    m.complete(p, SyntaxKind::UseQuery);
    Some(())
}

/// Whether an output clause starts here: `INTO OUTFILE`, `FORMAT name` or
/// `SETTINGS name`.
fn at_output_clause(p: &Parser<'_>) -> bool {
    common::at_into_outfile(p) || common::at_format(p) || common::at_settings(p)
}

/// The output clauses of a statement `ParserQueryWithOutput` reads, each
/// with the clause fallback, in any order and repeated, as clauses are.
fn output_clauses(p: &mut Parser<'_>) {
    loop {
        let parse = if common::at_into_outfile(p) {
            common::into_outfile_clause
        } else if common::at_format(p) {
            common::format_clause
        } else if common::at_settings(p) {
            |p: &mut Parser<'_>| common::settings_clause(p, &at_output_clause)
        } else {
            return;
        };
        fallback::clause(p, &at_output_clause, |p| parse(p).map(drop));
    }
}

#[cfg(test)]
mod tests;
