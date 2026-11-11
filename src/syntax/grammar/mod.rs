//! The grammar: functions that drive a [`Parser`] over one statement.
//!
//! # How the parts fit
//!
//! | Module | What it parses |
//! | --- | --- |
//! | this one | statement dispatch |
//! | [`fallback`] | the fallback ladder, bracketed and comma-separated lists, opaque regions |
//! | [`names`] | names, dotted names, Jinja expressions in name position |
//! | [`exprs`] | expressions, function calls, aliases |
//! | [`types`] | data types |
//! | [`common`] | expression lists, `ORDER BY` items, `SETTINGS`, `ON CLUSTER`, `IF [NOT] EXISTS` |
//! | [`select`] | `SELECT` queries and subqueries |
//! | [`create`] | `CREATE TABLE`, `CREATE MATERIALIZED VIEW` |
//! | [`insert`] | `INSERT`, with the `WITH` clause before it |
//! | [`probe`] | the `SELECT <list>` expression probe |
//!
//! # Writing grammar
//!
//! A grammar function takes `&mut Parser` and returns `Option`: `None`
//! means it failed, and it may then leave half-written events behind. Only
//! the [`fallback`] combinators clean up, by rolling back to where they
//! started, so every function that can fail runs inside one: an item inside
//! [`fallback::list_item`] (or a list built with
//! [`fallback::comma_separated`] or [`fallback::delimited`]), a clause
//! inside [`fallback::clause`], and the statement inside
//! [`fallback::statement`], which the dispatch already wraps around
//! [`select::statement`] and [`create::statement`]. Use `?` freely inside.
//!
//! **Looking ahead.** [`Parser::nth`] and [`Parser::at`] look at significant
//! tokens: trivia (whitespace, comments, `{% %}` and `{# #}`) is never seen,
//! so a Jinja block tag between two clauses is as invisible as a comment.
//! [`Parser::at_kw`], [`Parser::nth_at_kw`] and [`Parser::at_kws`] compare
//! keywords through the lexer's classification, which is case-insensitive
//! and never matches a word with a Jinja tag glued into it.
//!
//! **Consuming.** [`Parser::bump`] consumes a token as the lexer classified
//! it; a bare word consumed this way is a name, and keeps its case.
//! [`Parser::bump_keyword`], [`Parser::eat_kw`], [`Parser::eat_kws`] and
//! [`Parser::expect_kw`] consume a bare word in a **keyword slot**, which
//! makes it a [`Keyword`](SyntaxKind::Keyword) that the formatter will upper
//! case. Use them only where ClickHouse's parser matches a keyword, never
//! for a name that happens to be a keyword (`key`, `date`, `Null` as an
//! engine, `left(` as a function). [`Parser::expect`] and
//! [`Parser::expect_kw`] record what was expected when they fail; the
//! fallback that catches the failure reports it.
//!
//! **Nodes.** [`Parser::start`] opens a node and returns a
//! [`Marker`](crate::syntax::parser::Marker), which is either completed with
//! its kind or abandoned. [`CompletedMarker::precede`] wraps a finished
//! node in a new one, for syntax whose start is known only later (`a` in
//! `a + b`). The kinds are in [`SyntaxKind`]; each statement part has its
//! section there, and a node's [`Role`](crate::syntax::Role) (clause, list,
//! statement) is what the statistics and the printer go by.
//!
//! **Backtracking.** [`Parser::checkpoint`] and [`Parser::rollback`] return
//! to an earlier point, dropping the events, keyword slots and fallbacks
//! recorded since. Prefer lookahead; where an alternative must be tried, use
//! [`Parser::speculate`], which stops trying once the statement's budget of
//! re-read tokens is spent. Recursion goes through [`Parser::nested`]
//! (expressions, data types and subqueries already do), which fails beyond a
//! fixed depth; the failure then becomes a verbatim region.
//!
//! **Work.** Neither of those bounds the work: nested alternatives, each
//! within the depth limit and the budget, can still re-parse the same tokens
//! many times over. What bounds it is the statement's fuel, a fixed amount
//! per token that every look at a token spends; a statement that runs out
//! is kept verbatim (see [Work](crate::syntax::parser#work)). So the
//! grammar must spend little per token on real input: look ahead over a
//! bracket group with [`Parser::closer`] and [`Parser::group_items`] rather
//! than by counting brackets, consume openers with [`Parser::expect`], which
//! fails at once on a bracket that is never closed, and keep the
//! `steps_stay_linear` tests in `fallback/tests.rs` passing for the shapes
//! that tempt a grammar into re-parsing.
//!
//! **Verbatim.** Text that is kept as written by design, such as `OVER
//! (…)` or `INTO OUTFILE …`, is [`fallback::opaque_group`] or
//! [`fallback::opaque_until`]; it is not a parse failure.
//!
//! **Trivia.** Grammar functions never see or place trivia: the tree builder
//! puts it between siblings (see [`syntax`](crate::syntax#trivia)), so a
//! node is whatever its first and last significant tokens enclose.
//!
//! # The building blocks
//!
//! - Names: [`names::name`] (one identifier: a bare word whatever it spells,
//!   a quoted identifier, `{{ … }}`, `{x:Identifier}`) and
//!   [`names::compound_name`] (`db.table`). An engine, codec, setting or type
//!   name is a name, never a keyword: `ENGINE = Null` stays as written.
//!   [`insert::table_name`] is `name` or `db.name`, where ClickHouse reads
//!   a table as two identifiers (`parseDatabaseAndTableAsAST`).
//! - Expressions: [`exprs::expr`], and [`exprs::expr_with_alias`] with an
//!   [`AliasMode`]: `Implicit` for a `SELECT` list (an alias without `AS`
//!   cannot be one of [`exprs::RESTRICTED_ALIASES`]), `As` where only
//!   `AS alias` is allowed. [`exprs::alias`] adds an alias to a node already
//!   parsed, such as a table in `FROM`. [`exprs::arg_list`] parses `( … )`
//!   arguments after a name a caller has parsed.
//! - Data types: [`types::data_type`].
//! - Lists: [`common::expr_list`] and [`common::order_by_list`] for lists
//!   that end at a clause keyword, [`fallback::delimited`] for bracketed
//!   ones; for others, open a node and call [`fallback::comma_separated`].
//! - Clause parts: [`common::at_settings`] and [`common::settings_clause`]
//!   (`SETTINGS` opens a clause before a setting's name), [`common::on_cluster`],
//!   [`common::if_not_exists`], [`common::if_exists`].
//! - Queries: [`select::query`], [`select::subquery`] and
//!   [`select::at_subquery`], which the `SELECT` grammar fills in.
//!
//! # Adding a statement
//!
//! 1. Declare its node kinds in [`SyntaxKind`], in its section, with the
//!    right role: the statement's body is [`Role::Statement`](crate::syntax::Role),
//!    each clause [`Role::Clause`](crate::syntax::Role), each list
//!    [`Role::List`](crate::syntax::Role).
//! 2. Recognise it in an `at_statement` function by its leading keywords
//!    and call it from [`statement`] (`select`, `create` and `insert` are
//!    dispatched already).
//! 3. Parse its head, then its clauses in a loop: while the current token
//!    starts a clause, parse it with [`fallback::clause`], passing a
//!    predicate that recognises every clause keyword of the statement, so
//!    that clauses may come in any order and repeat. Lists inside a clause
//!    take the same predicate as their end, so that an item that does not
//!    parse stops at the next clause:
//!
//!    ```ignore
//!    fallback::clause(p, &at_clause, |p| {
//!        let m = p.start();
//!        p.eat_kws(&[K::Group, K::By]);
//!        common::expr_list(p, &at_clause, AliasMode::As)?;
//!        m.complete(p, SyntaxKind::GroupByClause);
//!        Some(())
//!    });
//!    ```
//!
//! 4. Test it in the module's `tests.rs` with tree snapshots (see
//!    `exprs/tests.rs` and `fallback/tests.rs`), including comments and
//!    Jinja between clauses, a clause that does not parse, and the words
//!    that are keywords in one place and names in another; and run
//!    `mise run corpus` and `mise run corpus:recase` for the statistics and
//!    the oracle's view of the keyword slots.

pub(crate) mod common;
pub(crate) mod create;
pub(crate) mod exprs;
pub(crate) mod fallback;
pub(crate) mod insert;
pub(crate) mod names;
pub(crate) mod probe;
pub(crate) mod select;
pub(crate) mod types;

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::Entry;
#[cfg(doc)]
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::exprs::AliasMode;
#[cfg(doc)]
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Parses the statement the parser is set to, as `entry` says.
pub(crate) fn statement(p: &mut Parser<'_>, entry: Entry) {
    if p.at_end() {
        return;
    }
    match entry {
        Entry::File => {
            if at_parallel_with(p) {
                fallback::verbatim_statement(p);
            } else if insert::at_statement(p) || (p.at_kw(K::With) && at_insert_after_with(p)) {
                fallback::statement(p, insert::statement);
            } else if select::at_statement(p) {
                fallback::statement(p, select::statement);
            } else if create::at_statement(p) {
                fallback::statement(p, create::statement);
            } else {
                fallback::verbatim_statement(p);
            }
        }
        Entry::SelectListProbe => probe::statement(p),
        Entry::Expression => fallback::statement(p, |p| {
            exprs::expr_with_alias(p, AliasMode::Implicit).map(drop)
        }),
        Entry::DataType => fallback::statement(p, |p| types::data_type(p).map(drop)),
    }
}

/// Whether the statement is two or more joined by `PARALLEL WITH`
/// (`ParserParallelWithQuery`), which the grammar does not parse. Such a
/// statement is kept verbatim whole rather than parsed in part, which would
/// record keyword slots that the grammar for the whole might not.
///
/// The words count outside brackets only, and not before `TOTALS`,
/// `ROLLUP`, `CUBE`, `FILL` or `TIES`, where `parallel` is a column that the
/// clause's `WITH` follows.
fn at_parallel_with(p: &Parser<'_>) -> bool {
    let mut n = 0;
    while p.nth(n).is_some() {
        if p.nth_at_kw(n, K::Parallel)
            && p.nth_at_kw(n + 1, K::With)
            && !matches!(
                p.nth_keyword(n + 2),
                Some(K::Totals | K::Rollup | K::Cube | K::Fill | K::Ties)
            )
        {
            return true;
        }
        n += p.step_len(n);
    }
    false
}

/// Whether a statement that starts with `WITH` is an `INSERT` after a `WITH`
/// clause (`WITH x AS (…) INSERT INTO t …`, which `ParserInsertQuery`
/// reads): `INSERT INTO` outside brackets, but not before `OUTFILE` and a
/// string, where `insert` is an alias.
fn at_insert_after_with(p: &Parser<'_>) -> bool {
    let mut n = 0;
    while p.nth(n).is_some() {
        if p.nth_at_kw(n, K::Insert)
            && p.nth_at_kw(n + 1, K::Into)
            && !(p.nth_at_kw(n + 2, K::Outfile) && p.nth_at(n + 3, T::StringLiteral))
        {
            return true;
        }
        n += p.step_len(n);
    }
    false
}
