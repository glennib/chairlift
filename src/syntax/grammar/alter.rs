//! `ALTER TABLE`, `ALTER DATABASE` and the lightweight `DELETE` and
//! `UPDATE`, as ClickHouse's `ParserAlterQuery`, `ParserDeleteQuery` and
//! `ParserUpdateQuery` read them (`src/Parsers/ParserAlterQuery.cpp`).
//!
//! An `ALTER` is its head (the keywords, the name, `ON CLUSTER`), an
//! [`AlterCommandList`](SyntaxKind::AlterCommandList) with one
//! [`AlterCommand`](SyntaxKind::AlterCommand) per command, each with the
//! item fallback, and output clauses. A command reuses what `CREATE TABLE`
//! parses: column, index, projection and constraint definitions, `ORDER BY`,
//! `SAMPLE BY`, `TTL`, `SETTINGS` lists, `REFRESH` and `SQL SECURITY`.
//!
//! # Commas
//!
//! A list inside a command reads on after a comma, whatever word follows, as
//! ClickHouse's `ParserList` does: in `MODIFY SETTING a = 1, b = 2`, `MODIFY
//! TTL …, …`, `UPDATE a = 1, b = 2`, `DELETE IN PARTITION 1, 2` and the
//! columns and types of `STATISTICS`, the comma is the list's, so such a
//! command can only be followed by another as the last of its list allows.
//! `MODIFY QUERY` takes the rest of the statement.
//!
//! # Keyword slots
//!
//! The words of a command's phrase (`ADD COLUMN`, `DROP DETACHED PART`,
//! `APPLY DELETED MASK`), `IF [NOT] EXISTS`, `FIRST` and `AFTER` after a
//! definition, `TO` of `RENAME COLUMN`, `IN PARTITION`, `FROM` and `TO
//! DISK|VOLUME|TABLE|SHARD`, `WITH NAME`, and `REMOVE` and the property it
//! names. Names are names whatever they spell: `DROP COLUMN index`, `RENAME
//! COLUMN to TO from`, `ADD COLUMN first Int8 FIRST`, `AFTER after`.

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::create;
use crate::syntax::grammar::create::elements;
use crate::syntax::grammar::create::elements::ColumnForm;
use crate::syntax::grammar::create::storage;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::fallback::Stop;
use crate::syntax::grammar::insert::table_name;
use crate::syntax::grammar::names;
use crate::syntax::grammar::select;
use crate::syntax::grammar::select::Level;
use crate::syntax::grammar::simple;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Whether the statement at the current token is one of this module's:
/// `ALTER [TEMPORARY] TABLE`, `ALTER DATABASE`, `DELETE FROM` or `UPDATE`
/// and a name.
pub(crate) fn at_statement(p: &Parser<'_>) -> bool {
    match p.nth_keyword(0) {
        Some(K::Alter) => {
            matches!(p.nth_keyword(1), Some(K::Table | K::Database))
                || (p.nth_at_kw(1, K::Temporary) && p.nth_at_kw(2, K::Table))
        }
        Some(K::Delete) => p.nth_at_kw(1, K::From),
        Some(K::Update) => names::at_name(p, 1),
        _ => false,
    }
}

/// A statement of this module's, up to the end of the statement.
pub(crate) fn statement(p: &mut Parser<'_>) -> Option<()> {
    match p.nth_keyword(0) {
        Some(K::Alter) => alter(p),
        Some(K::Delete) => delete(p),
        Some(K::Update) => update(p),
        _ => p.expected("a statement"),
    }
}

/// An [`AlterQuery`](SyntaxKind::AlterQuery).
fn alter(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    let database = p.at_kw(K::Database);
    if database {
        p.bump_keyword();
        names::name(p)?;
    } else {
        p.eat_kw(K::Temporary);
        p.expect_kw(K::Table)?;
        table_name(p)?;
    }
    if p.at_kw(K::On) {
        common::on_cluster(p)?;
    }
    let list = p.start();
    let shape =
        fallback::comma_separated(p, &simple::at_output_clause, &mut |p| command(p, database));
    if shape.items == 0 {
        list.abandon(p);
        return p.expected("an ALTER command");
    }
    list.complete(p, SyntaxKind::AlterCommandList);
    simple::output_clauses(p);
    m.complete(p, SyntaxKind::AlterQuery);
    Some(())
}

/// One [`AlterCommand`](SyntaxKind::AlterCommand), in brackets if it starts
/// with one.
fn command(p: &mut Parser<'_>, database: bool) -> Option<()> {
    let m = p.start();
    let bracketed = p.eat(T::OpeningRoundBracket);
    if database {
        database_command(p)?;
    } else {
        table_command(p)?;
    }
    if bracketed {
        p.expect(T::ClosingRoundBracket)?;
    }
    m.complete(p, SyntaxKind::AlterCommand);
    Some(())
}

/// Where a list inside a command ends: at an output clause of the
/// statement.
fn at_end_of_list(p: &Parser<'_>) -> bool {
    simple::at_output_clause(p)
}

/// A command of `ALTER DATABASE`: `MODIFY SETTING` or `MODIFY COMMENT`.
fn database_command(p: &mut Parser<'_>) -> Option<()> {
    if p.eat_kws(&[K::Modify, K::Setting]) {
        common::settings_list(p, &at_end_of_list)?;
    } else if p.eat_kws(&[K::Modify, K::Comment]) {
        exprs::string(p)?;
    } else {
        return p.expected("MODIFY SETTING or MODIFY COMMENT");
    }
    Some(())
}

/// Consumes the next `words` tokens as keywords.
fn phrase(p: &mut Parser<'_>, words: usize) {
    for _ in 0..words {
        p.bump_keyword();
    }
}

/// A command of `ALTER TABLE`, by its leading keywords, in the forms
/// `ParserAlterCommand` reads.
fn table_command(p: &mut Parser<'_>) -> Option<()> {
    let first = p.nth_keyword(0);
    let second = p.nth_keyword(1);
    match (first?, second) {
        (K::Add, Some(K::Column)) => {
            phrase(p, 2);
            common::if_not_exists(p);
            elements::column(p, ColumnForm::Add)?;
            position(p)?;
        }
        (K::Add, Some(K::Index)) => {
            phrase(p, 2);
            common::if_not_exists(p);
            elements::index(p, false)?;
            position(p)?;
        }
        (K::Add, Some(K::Projection)) => {
            phrase(p, 2);
            common::if_not_exists(p);
            elements::projection(p, false)?;
            position(p)?;
        }
        (K::Add, Some(K::Constraint)) => {
            phrase(p, 2);
            common::if_not_exists(p);
            elements::constraint(p, false)?;
        }
        (K::Add, Some(K::Statistics)) => {
            phrase(p, 2);
            common::if_not_exists(p);
            statistics(p, true)?;
        }
        (K::Modify, Some(K::Statistics)) => {
            phrase(p, 2);
            statistics(p, true)?;
        }
        (K::Drop | K::Clear | K::Materialize, Some(K::Statistics)) => {
            phrase(p, 2);
            if first != Some(K::Drop) && p.eat_kw(K::All) {
                return Some(());
            }
            common::if_exists(p);
            statistics(p, false)?;
            if first != Some(K::Drop) {
                in_partition(p, false)?;
            }
        }
        (K::Drop, Some(K::Column | K::Index | K::Projection | K::Constraint)) => {
            phrase(p, 2);
            common::if_exists(p);
            names::compound_name(p)?;
        }
        (K::Materialize, Some(K::Column)) => {
            phrase(p, 2);
            names::compound_name(p)?;
            in_partition(p, false)?;
        }
        (K::Clear, Some(K::Column | K::Index | K::Projection))
        | (K::Materialize, Some(K::Index | K::Projection)) => {
            phrase(p, 2);
            common::if_exists(p);
            names::compound_name(p)?;
            in_partition(p, false)?;
        }
        (K::Materialize, Some(K::Ttl))
        | (K::Rewrite, Some(K::Parts))
        | (K::Apply, Some(K::Patches)) => {
            phrase(p, 2);
            in_partition(p, false)?;
        }
        (K::Apply, Some(K::Deleted)) => {
            phrase(p, 2);
            p.expect_kw(K::Mask)?;
            in_partition(p, false)?;
        }
        (K::Modify | K::Alter, Some(K::Column)) => {
            phrase(p, 2);
            common::if_exists(p);
            let form = if first == Some(K::Modify) {
                ColumnForm::Modify
            } else {
                ColumnForm::Alter
            };
            elements::column(p, form)?;
            column_change(p)?;
        }
        (K::Rename, Some(K::Column)) => {
            phrase(p, 2);
            common::if_exists(p);
            names::compound_name(p)?;
            p.expect_kw(K::To)?;
            names::compound_name(p)?;
        }
        (K::Comment, Some(K::Column)) => {
            phrase(p, 2);
            common::if_exists(p);
            names::compound_name(p)?;
            exprs::string(p)?;
        }
        (K::Modify, Some(K::Order)) if p.nth_at_kw(2, K::By) => {
            p.bump_keyword();
            storage::order_by(p)?;
        }
        (K::Modify, Some(K::Sample)) if p.nth_at_kw(2, K::By) => {
            p.bump_keyword();
            storage::key(p, SyntaxKind::SampleByClause)?;
        }
        (K::Remove, Some(K::Sample)) => {
            phrase(p, 2);
            p.expect_kw(K::By)?;
        }
        (K::Modify, Some(K::Ttl)) => {
            p.bump_keyword();
            storage::ttl(p, &at_end_of_list)?;
        }
        (K::Remove, Some(K::Ttl)) => phrase(p, 2),
        (K::Modify | K::Reset, Some(K::Setting)) => {
            phrase(p, 2);
            if first == Some(K::Modify) {
                common::settings_list(p, &at_end_of_list)?;
            } else {
                name_list(p, &at_end_of_list)?;
            }
        }
        (K::Modify, Some(K::Query)) => {
            phrase(p, 2);
            select::query(p, Level::Nested)?;
        }
        (K::Modify, Some(K::Sql | K::Definer)) => {
            p.bump_keyword();
            create::sql_security(p)?;
        }
        (K::Modify, Some(K::Refresh)) => {
            p.bump_keyword();
            create::refresh(p)?;
        }
        (K::Modify, Some(K::Comment)) => {
            phrase(p, 2);
            exprs::string(p)?;
        }
        (K::Modify, Some(K::Projection)) => {
            phrase(p, 2);
            common::if_exists(p);
            elements::projection(p, false)?;
        }
        (K::Modify, Some(K::Constraint)) => {
            phrase(p, 2);
            common::if_exists(p);
            elements::constraint(p, false)?;
        }
        (K::Delete, _) => {
            p.bump_keyword();
            in_partition(p, true)?;
            where_clause(p)?;
        }
        (K::Update, _) => {
            p.bump_keyword();
            assignment_list(p)?;
            in_partition(p, true)?;
            where_clause(p)?;
        }
        _ => partition_command(p)?,
    }
    Some(())
}

/// The partition and part commands: `DROP`, `DETACH`, `ATTACH`, `FORGET`,
/// `MOVE`, `REPLACE`, `FETCH`, `FREEZE` and `UNFREEZE`, and `EXECUTE`.
fn partition_command(p: &mut Parser<'_>) -> Option<()> {
    let verb = p.nth_keyword(0);
    match (verb?, p.nth_keyword(1)) {
        (K::Drop, Some(K::Detached)) => {
            phrase(p, 2);
            partition_or_part(p)?;
        }
        (K::Drop | K::Detach | K::Attach | K::Move | K::Fetch, Some(K::Partition | K::Part))
        | (K::Forget | K::Replace, Some(K::Partition)) => {
            p.bump_keyword();
            let part = partition_or_part(p)?;
            match verb {
                Some(K::Attach) if p.eat_kw(K::From) => {
                    if part {
                        exprs::string(p)?;
                    } else {
                        table_name(p)?;
                    }
                }
                Some(K::Replace) => {
                    p.expect_kw(K::From)?;
                    table_name(p)?;
                }
                Some(K::Fetch) => {
                    p.expect_kw(K::From)?;
                    exprs::string(p)?;
                }
                Some(K::Move) => move_destination(p, part)?,
                _ => {}
            }
        }
        (K::Freeze | K::Unfreeze, _) => {
            p.bump_keyword();
            if p.at_kw(K::Partition) {
                simple::partition(p)?;
            }
            if p.eat_kws(&[K::With, K::Name]) {
                exprs::string(p)?;
            } else if verb == Some(K::Unfreeze) {
                return p.expected("WITH NAME");
            }
        }
        (K::Execute, _) => {
            p.bump_keyword();
            let call = names::name(p)?.precede(p);
            exprs::arg_list(p)?;
            call.complete(p, SyntaxKind::FunctionCall);
        }
        _ => return p.expected("an ALTER command"),
    }
    Some(())
}

/// `PARTITION` and a partition, as a [`Partition`](SyntaxKind::Partition),
/// or `PART` and the part's name; returns whether it was a part.
fn partition_or_part(p: &mut Parser<'_>) -> Option<bool> {
    if p.at_kw(K::Partition) {
        simple::partition(p)?;
        return Some(false);
    }
    p.expect_kw(K::Part)?;
    part_name(p)?;
    Some(true)
}

/// A part's name: a string or a `{name:String}` parameter
/// (`ParserStringAndSubstitution`).
fn part_name(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if names::at_query_parameter(p, 0) {
        exprs::query_parameter(p)
    } else {
        exprs::string(p)
    }
}

/// Where `MOVE PARTITION` or `MOVE PART` moves to: `TO DISK '…'`, `TO
/// VOLUME '…'`, `TO TABLE [db.]t` for a partition, `TO SHARD '…'` for a
/// part.
fn move_destination(p: &mut Parser<'_>, part: bool) -> Option<()> {
    p.expect_kw(K::To)?;
    match p.nth_keyword(0) {
        Some(K::Disk | K::Volume) => {
            p.bump_keyword();
            exprs::string(p)?;
        }
        Some(K::Table) if !part => {
            p.bump_keyword();
            table_name(p)?;
        }
        Some(K::Shard) if part => {
            p.bump_keyword();
            exprs::string(p)?;
        }
        _ => return p.expected("DISK, VOLUME, TABLE or SHARD"),
    }
    Some(())
}

/// `FIRST` or `AFTER name`, if either follows a definition.
fn position(p: &mut Parser<'_>) -> Option<()> {
    if !p.eat_kw(K::First) && p.eat_kw(K::After) {
        names::compound_name(p)?;
    }
    Some(())
}

/// What follows the column of `MODIFY COLUMN` or `ALTER COLUMN`: `REMOVE`
/// and a property, `MODIFY SETTING` and settings, `RESET SETTING` and
/// names, `ADD ENUM VALUES (…)`, or `FIRST` or `AFTER name`.
fn column_change(p: &mut Parser<'_>) -> Option<()> {
    if p.eat_kw(K::Remove) {
        if !matches!(
            p.nth_keyword(0),
            Some(
                K::Default
                    | K::Materialized
                    | K::Alias
                    | K::Comment
                    | K::Codec
                    | K::Ttl
                    | K::Settings
            )
        ) {
            return p.expected("the property to remove");
        }
        p.bump_keyword();
    } else if p.eat_kws(&[K::Modify, K::Setting]) {
        common::settings_list(p, &at_end_of_list)?;
    } else if p.eat_kws(&[K::Reset, K::Setting]) {
        name_list(p, &at_end_of_list)?;
    } else if p.eat_kws(&[K::Add, K::Enum, K::Values]) {
        fallback::delimited(
            p,
            SyntaxKind::ExprList,
            T::OpeningRoundBracket,
            T::ClosingRoundBracket,
            &mut |p| exprs::expr(p).map(drop),
        )?;
    } else {
        position(p)?;
    }
    Some(())
}

/// Names separated by commas, as a [`NameList`](SyntaxKind::NameList),
/// with items that end at `ends`.
fn name_list(p: &mut Parser<'_>, ends: Stop<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let shape = fallback::comma_separated(p, ends, &mut |p| names::name(p).map(drop));
    if shape.items == 0 {
        m.abandon(p);
        return p.expected("a name");
    }
    Some(m.complete(p, SyntaxKind::NameList))
}

/// A [`StatisticsDef`](SyntaxKind::StatisticsDef): columns, and with
/// `types` `TYPE` and the statistics kinds (`ParserStatisticsDeclaration`).
fn statistics(p: &mut Parser<'_>, types: bool) -> Option<CompletedMarker> {
    let m = p.start();
    let at_type = |p: &Parser<'_>| p.at_kw(K::Type) || at_end_of_list(p);
    let at_partition = |p: &Parser<'_>| p.at_kws(&[K::In, K::Partition]) || at_end_of_list(p);
    name_list(p, if types { &at_type } else { &at_partition })?;
    if types {
        p.expect_kw(K::Type)?;
        let list = p.start();
        let shape = fallback::comma_separated(p, &at_end_of_list, &mut |p| {
            create::name_with_args(p).map(drop)
        });
        if shape.items == 0 {
            list.abandon(p);
            return p.expected("a statistics kind");
        }
        list.complete(p, SyntaxKind::ExprList);
    }
    Some(m.complete(p, SyntaxKind::StatisticsDef))
}

/// `IN PARTITION` and a partition as an
/// [`InPartition`](SyntaxKind::InPartition), if it follows; with `several`,
/// as for a mutation, partitions separated by commas.
fn in_partition(p: &mut Parser<'_>, several: bool) -> Option<()> {
    if !p.at_kws(&[K::In, K::Partition]) {
        return Some(());
    }
    let m = p.start();
    phrase(p, 2);
    simple::partition_value(p)?;
    while several && p.at(T::Comma) {
        p.bump();
        simple::partition_value(p)?;
    }
    m.complete(p, SyntaxKind::InPartition);
    Some(())
}

/// `WHERE expr` as a [`WhereClause`](SyntaxKind::WhereClause).
fn where_clause(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.expect_kw(K::Where)?;
    exprs::expr(p)?;
    m.complete(p, SyntaxKind::WhereClause);
    Some(())
}

/// The assignments of `UPDATE`, as an
/// [`AssignmentList`](SyntaxKind::AssignmentList) that ends at `IN
/// PARTITION` or `WHERE`.
fn assignment_list(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let ends = |p: &Parser<'_>| p.at_kw(K::Where) || p.at_kws(&[K::In, K::Partition]);
    let shape = fallback::comma_separated(p, &ends, &mut |p| storage::assignment(p).map(drop));
    if shape.items == 0 {
        m.abandon(p);
        return p.expected("an assignment");
    }
    Some(m.complete(p, SyntaxKind::AssignmentList))
}

/// Any `IN PARTITION` and the `WHERE` clause, then any
/// `SETTINGS`: the end of `DELETE` and `UPDATE`.
fn mutation_tail(p: &mut Parser<'_>) -> Option<()> {
    in_partition(p, true)?;
    where_clause(p)?;
    if common::at_settings(p) {
        common::settings_clause(p, &fallback::never)?;
    }
    Some(())
}

/// A [`DeleteQuery`](SyntaxKind::DeleteQuery).
fn delete(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.expect_kw(K::From)?;
    table_name(p)?;
    if p.at_kw(K::On) {
        common::on_cluster(p)?;
    }
    mutation_tail(p)?;
    m.complete(p, SyntaxKind::DeleteQuery);
    Some(())
}

/// An [`UpdateQuery`](SyntaxKind::UpdateQuery).
fn update(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    table_name(p)?;
    if p.at_kw(K::On) {
        common::on_cluster(p)?;
    }
    let set = p.start();
    p.expect_kw(K::Set)?;
    assignment_list(p)?;
    set.complete(p, SyntaxKind::UpdateSetClause);
    mutation_tail(p)?;
    m.complete(p, SyntaxKind::UpdateQuery);
    Some(())
}

#[cfg(test)]
mod tests;
