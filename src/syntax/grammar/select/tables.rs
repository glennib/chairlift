//! `FROM`, the tables in it, and joins (`ParserTablesInSelectQuery`).

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::exprs::AliasMode;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::names;
use crate::syntax::grammar::select;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Which aliases without `AS` the tables take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Aliases {
    /// Any word but the restricted ones ([`exprs::RESTRICTED_ALIASES`]),
    /// `select` included: `FROM t select WHERE 1` names `t` `select`.
    Implicit,
    /// As [`Implicit`](Self::Implicit), but `select` only where it cannot
    /// start the `SELECT` clause, as in the `FROM`-first form ClickHouse
    /// settles it (`ParserSelectQuery`): it parses the tables again without
    /// such aliases if they end with a bare `select` alias and no `SELECT`
    /// follows. So `FROM t select x` selects `x` from `t`, while in `FROM t
    /// select FINAL` and `FROM t select SELECT x` the alias stands.
    FromFirst,
    /// None: the tables of joins in brackets, which ClickHouse parses with
    /// `ParserTablesInSelectQuery(false)`.
    None,
}

/// `FROM` and a [`TableList`](SyntaxKind::TableList).
pub(super) fn from_clause(p: &mut Parser<'_>, aliases: Aliases) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    super::list(p, SyntaxKind::TableList, &mut |p| {
        table_expr(p, aliases, false).map(drop)
    })?;
    m.complete(p, SyntaxKind::FromClause);
    Some(())
}

/// A [`TableExpr`](SyntaxKind::TableExpr) (`ParserTableExpression`), with
/// aliases as `aliases` allows; `condition` says whether the table is joined
/// with `ON` or `USING`.
fn table_expr(p: &mut Parser<'_>, aliases: Aliases, condition: bool) -> Option<CompletedMarker> {
    let m = p.start();
    // A table named `from` takes no alias without `AS`, as in
    // `ParserWithOptionalAlias`.
    let lone_from =
        names::at_from(p, 0) && !matches!(p.nth(1), Some(T::Dot | T::OpeningRoundBracket));
    let table = table(p)?;
    let implicit = !lone_from
        && match aliases {
            Aliases::None => false,
            Aliases::FromFirst if p.at_kw(K::Select) => tables_go_on(p, 1, condition),
            Aliases::Implicit | Aliases::FromFirst => true,
        };
    let mode = if implicit {
        AliasMode::Implicit
    } else {
        AliasMode::As
    };
    exprs::alias(p, table, mode)?;
    // `t (a, b)` is a table function, so column aliases can only follow an
    // alias, a table function or a subquery.
    if p.at(T::OpeningRoundBracket) {
        super::name_list(p)?;
    }
    p.eat_kw(K::Final);
    if p.at_kw(K::Sample) && p.nth_at(1, T::Number) {
        let s = p.start();
        p.bump_keyword();
        exprs::expr(p)?;
        if p.eat_kw(K::Offset) {
            exprs::expr(p)?;
        }
        s.complete(p, SyntaxKind::SampleClause);
    }
    if p.at_kw(K::Stream) {
        stream(p)?;
    }
    // The query's column aliases, which `ParserSelectQuery` reads after the
    // tables: `FROM t FINAL (a, b)`.
    if p.at(T::OpeningRoundBracket) {
        super::name_list(p)?;
    }
    Some(m.complete(p, SyntaxKind::TableExpr))
}

/// Whether a table can start at the `n`th token (`ParserTableExpression`): a
/// name, whatever word it is, a file name as a string, or brackets that are
/// closed and may hold a query or joins, which is what `SELECT`, `WITH`,
/// `FROM`, `VALUES`, `EXPLAIN` or `(` first, or a comma or `JOIN` outside
/// further brackets, say. `(1)` holds neither, so `from (1)` calls a
/// function named `from`. It looks no further into the brackets, so that
/// nested brackets cost one look each.
pub(super) fn at_table(p: &Parser<'_>, n: usize) -> bool {
    match p.nth(n) {
        Some(T::OpeningRoundBracket) => {
            p.closer(n).is_some()
                && (p.nth_at(n + 1, T::OpeningRoundBracket)
                    || matches!(
                        p.nth_keyword(n + 1),
                        Some(K::Select | K::With | K::From | K::Values | K::Explain)
                    )
                    || p.group_items(n)
                        .any(|i| p.nth_at(i, T::Comma) || p.nth_at_kw(i, K::Join)))
        }
        Some(T::StringLiteral) => true,
        _ => names::at_name(p, n),
    }
}

/// The table itself: a subquery, joins in parentheses, a table function, a
/// name, or a file name as a string.
fn table(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    match p.nth(0) {
        Some(T::OpeningRoundBracket) if at_parenthesized_query(p) => select::subquery(p),
        Some(T::OpeningRoundBracket) => p.nested(paren_join),
        Some(T::StringLiteral) => exprs::literal(p),
        _ => {
            let view = view_at(p, 0, true);
            let name = names::compound_name(p)?;
            if p.at(T::OpeningRoundBracket) {
                table_function(p, name, view)
            } else {
                Some(name)
            }
        }
    }
}

/// Whether the `(` at the current token holds a query: a subquery, or a set
/// operation between queries in parentheses, `((SELECT 1) UNION ALL
/// (SELECT 2))`. Otherwise it holds joins. A run of brackets is looked at
/// only as deep as the parse can still nest, since a deeper query fails
/// either way; each level of the run looks again, so a longer bound would
/// cost the square of the run.
fn at_parenthesized_query(p: &Parser<'_>) -> bool {
    (0..p.depth_left())
        .take_while(|&n| p.nth_at(n, T::OpeningRoundBracket))
        .any(|n| select::at_subquery(p, n))
}

/// Whether a table function takes a query as its argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueryArgument {
    None,
    /// `view(SELECT …)`, `eval(SELECT …)`, `obfuscate(SELECT …)`.
    Query,
    /// `viewIfPermitted(SELECT … ELSE f(…))`.
    QueryOrElse,
    /// `exists(SELECT …)` as the table function itself: a
    /// [`Subquery`](SyntaxKind::Subquery), as in an expression.
    Exists,
}

/// What `view(` and its relatives at the `n`th token take, as ClickHouse's
/// layers for table functions (`getFunctionLayer`) read them: a query if
/// one follows the bracket. Where the call is the table function itself
/// (`table`), `exists(SELECT …)` takes one too (`ExistsLayer`); inside a
/// table function's arguments, `exists(…)` is an operand that an operator
/// may follow, which the expression grammar reads.
fn view_at(p: &Parser<'_>, n: usize, table: bool) -> QueryArgument {
    if !(p.nth_at(n, T::BareWord) && p.nth_at(n + 1, T::OpeningRoundBracket)) {
        return QueryArgument::None;
    }
    let name = p.nth_text(n);
    if table && name.eq_ignore_ascii_case("exists") {
        return if select::at_subquery(p, n + 1) {
            QueryArgument::Exists
        } else {
            QueryArgument::None
        };
    }
    if !at_query(p, n + 2) {
        QueryArgument::None
    } else if ["view", "eval", "obfuscate"]
        .iter()
        .any(|v| v.eq_ignore_ascii_case(name))
    {
        QueryArgument::Query
    } else if name.eq_ignore_ascii_case("viewIfPermitted") {
        QueryArgument::QueryOrElse
    } else {
        QueryArgument::None
    }
}

/// Whether the `n`th token starts a query without parentheses.
fn at_query(p: &Parser<'_>, n: usize) -> bool {
    p.nth_at_kw(n, K::Select) || p.nth_at_kw(n, K::With) || select::at_bracketed_from_first(p, n)
}

/// A table function at the current token, `remote('host', t)` or
/// `view(SELECT …)`, as ClickHouse reads one after `FROM` and after `CREATE
/// TABLE … AS` alike (`ParserFunction` for a table function).
pub(crate) fn table_function_call(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let view = view_at(p, 0, true);
    let name = names::name(p)?;
    table_function(p, name, view)
}

/// A table function whose name, `name`, has been parsed: a
/// [`FunctionCall`](SyntaxKind::FunctionCall) with an
/// [`ArgList`](SyntaxKind::ArgList). `view` says whether its argument is a
/// query; a nested call is looked at on its own.
fn table_function(
    p: &mut Parser<'_>,
    name: CompletedMarker,
    view: QueryArgument,
) -> Option<CompletedMarker> {
    let m = name.precede(p);
    if view == QueryArgument::Exists {
        select::subquery(p)?;
    } else {
        p.nested(|p| table_function_args(p, view))?;
    }
    Some(m.complete(p, SyntaxKind::FunctionCall))
}

/// The arguments of a table function: a query for `view` and its relatives,
/// otherwise expressions, a nested `view(SELECT …)`, or `SETTINGS a = 1, …`,
/// which takes the rest of the list.
fn table_function_args(p: &mut Parser<'_>, view: QueryArgument) -> Option<()> {
    let m = p.start();
    p.expect(T::OpeningRoundBracket)?;
    p.with_no_in(false, |p| {
        p.with_else_ending_query(view == QueryArgument::QueryOrElse, |p| {
            table_function_body(p, view)
        })
    })?;
    p.expect(T::ClosingRoundBracket)?;
    m.complete(p, SyntaxKind::ArgList);
    Some(())
}

/// What lies between the brackets of a table function's arguments.
fn table_function_body(p: &mut Parser<'_>, view: QueryArgument) -> Option<()> {
    if view == QueryArgument::None {
        fallback::comma_separated(p, &fallback::never, &mut |p| {
            let nested = view_at(p, 0, false);
            if nested != QueryArgument::None {
                let name = names::name(p)?;
                table_function(p, name, nested).map(drop)
            } else if common::at_settings(p) {
                common::settings_clause(p, &fallback::never).map(drop)
            } else {
                exprs::expr_with_alias(p, AliasMode::As).map(drop)
            }
        });
        Some(())
    } else {
        select::query(p, select::Level::Nested)?;
        if view == QueryArgument::QueryOrElse && p.eat_kw(K::Else) {
            let name = names::name(p)?;
            table_function(p, name, QueryArgument::None)?;
        }
        Some(())
    }
}

/// `( tables joins )`, which ClickHouse reads as a subquery `(SELECT * FROM
/// tables joins)`: more than one table, or a join.
fn paren_join(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect(T::OpeningRoundBracket)?;
    // The tables take no item fallback: these brackets nest, and a fallback
    // at every level would skip the same rest of the statement once per
    // level when the innermost fails.
    let tables = p.start();
    let mut count = 0;
    loop {
        table_expr(p, Aliases::None, false)?;
        count += 1;
        if !p.eat(T::Comma) {
            break;
        }
    }
    tables.complete(p, SyntaxKind::TableList);
    let mut joined = count > 1;
    let mut after_join = false;
    loop {
        if at_array_join(p, 0) {
            array_join_clause(p)?;
        } else if at_join(p) {
            join_clause(p, Aliases::None)?;
        } else if after_join && p.at(T::Comma) {
            comma_join(p, Aliases::None)?;
        } else {
            break;
        }
        after_join = true;
        joined = true;
    }
    if !joined {
        return p.expected("a join");
    }
    p.expect(T::ClosingRoundBracket)?;
    Some(m.complete(p, SyntaxKind::ParenJoin))
}

/// `STREAM [BOUNDED] [UNORDERED] [CURSOR {…}] [WATERMARK FOR c AS e [IDLE
/// TIMEOUT INTERVAL n unit]]` (`ParserStreamSettings`).
fn stream(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.eat_kw(K::Bounded);
    p.eat_kw(K::Unordered);
    if p.at_kw(K::Cursor) && p.nth_at(1, T::OpeningCurlyBrace) {
        p.bump_keyword();
        cursor(p)?;
    }
    if p.eat_kw(K::Watermark) {
        p.expect_kw(K::For)?;
        names::name(p)?;
        p.expect_kw(K::As)?;
        exprs::expr(p)?;
        if p.eat_kws(&[K::Idle, K::Timeout]) {
            exprs::expr(p)?;
        }
    }
    m.complete(p, SyntaxKind::StreamClause);
    Some(())
}

/// `{'0': {'block_number': 10}, …}`, where `STREAM CURSOR` resumes: string
/// keys, and unsigned numbers or nested maps as values
/// (`parseCursorObject`).
fn cursor(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    p.nested(|p| {
        exprs::map_literal(p, &mut |p| {
            if p.at(T::OpeningCurlyBrace) {
                cursor(p).map(drop)
            } else if p.at(T::Number) {
                exprs::literal(p).map(drop)
            } else {
                p.expected("a number")
            }
        })
    })
}

/// The number of tokens of a join's keywords up to `JOIN`, if a join starts
/// at the `start`th token: `[GLOBAL|LOCAL] [NATURAL] [ANY|ALL|ASOF|SEMI|
/// ANTI|ONLY] [INNER|LEFT|RIGHT|FULL|CROSS|PASTE] [ANY|…] [OUTER] JOIN`.
pub(super) fn join_keywords(p: &Parser<'_>, start: usize) -> Option<usize> {
    let strictness = |k| {
        matches!(
            k,
            Some(K::Any | K::All | K::Asof | K::Semi | K::Anti | K::Only)
        )
    };
    let mut n = start;
    // Every word of a join is a keyword.
    p.nth_keyword(n)?;
    if matches!(p.nth_keyword(n), Some(K::Global | K::Local)) {
        n += 1;
    }
    if p.nth_at_kw(n, K::Natural) {
        n += 1;
    }
    if strictness(p.nth_keyword(n)) {
        n += 1;
    }
    if matches!(
        p.nth_keyword(n),
        Some(K::Inner | K::Left | K::Right | K::Full | K::Cross | K::Paste)
    ) {
        n += 1;
    }
    if strictness(p.nth_keyword(n)) {
        n += 1;
    }
    if p.nth_at_kw(n, K::Outer) {
        n += 1;
    }
    p.nth_at_kw(n, K::Join).then_some(n + 1 - start)
}

/// Whether a join starts at the current token.
pub(super) fn at_join(p: &Parser<'_>) -> bool {
    join_keywords(p, 0).is_some()
}

/// Whether the tables go on at the `n`th token, after a table's alias: with
/// column aliases, `FINAL`, `SAMPLE`, `STREAM`, another table or join, or,
/// if `condition` is set, the join's `ON` or `USING`; or whether `SELECT`
/// follows.
fn tables_go_on(p: &Parser<'_>, n: usize, condition: bool) -> bool {
    p.nth_at(n, T::Comma)
        || (p.nth_at(n, T::OpeningRoundBracket)
            && names::at_name(p, n + 1)
            && matches!(p.nth(n + 2), Some(T::Comma | T::ClosingRoundBracket)))
        || matches!(p.nth_keyword(n), Some(K::Select | K::Final | K::Stream))
        || (p.nth_at_kw(n, K::Sample) && p.nth_at(n + 1, T::Number))
        || (condition && matches!(p.nth_keyword(n), Some(K::On | K::Using)))
        || join_keywords(p, n).is_some()
        || at_array_join(p, n)
}

/// A [`JoinClause`](SyntaxKind::JoinClause): its keywords, any `LATERAL`, a
/// table with aliases as `aliases` allows, and an `ON` or `USING` condition
/// if one follows.
pub(super) fn join_clause(p: &mut Parser<'_>, aliases: Aliases) -> Option<()> {
    let m = p.start();
    let keywords = join_keywords(p, 0)?;
    // `CROSS`, `PASTE` and `NATURAL` joins take no condition.
    let condition =
        !(0..keywords).any(|n| matches!(p.nth_keyword(n), Some(K::Cross | K::Paste | K::Natural)));
    for _ in 0..keywords {
        p.bump_keyword();
    }
    // `LATERAL` is a keyword only before a subquery; `JOIN lateral ON …`
    // joins a table named `lateral`.
    if p.at_kw(K::Lateral) && select::at_subquery(p, 1) {
        p.bump_keyword();
    }
    table_expr(p, aliases, condition)?;
    if p.at_kw(K::On) {
        let on = p.start();
        p.bump_keyword();
        exprs::expr(p)?;
        on.complete(p, SyntaxKind::JoinOn);
    } else if p.at_kw(K::Using) {
        let using = p.start();
        p.bump_keyword();
        if p.at(T::OpeningRoundBracket) {
            super::paren_expr_list(p)?;
        } else {
            super::expr_list(p)?;
        }
        using.complete(p, SyntaxKind::JoinUsing);
    }
    m.complete(p, SyntaxKind::JoinClause);
    Some(())
}

/// `, table` after a join, as a [`JoinClause`](SyntaxKind::JoinClause).
pub(super) fn comma_join(p: &mut Parser<'_>, aliases: Aliases) -> Option<()> {
    let m = p.start();
    p.expect(T::Comma)?;
    table_expr(p, aliases, false)?;
    m.complete(p, SyntaxKind::JoinClause);
    Some(())
}

/// Whether `[LEFT | INNER] ARRAY JOIN` starts at the `n`th token.
pub(super) fn at_array_join(p: &Parser<'_>, n: usize) -> bool {
    let n = n + usize::from(matches!(p.nth_keyword(n), Some(K::Left | K::Inner)));
    p.nth_at_kw(n, K::Array) && p.nth_at_kw(n + 1, K::Join)
}

/// `[LEFT | INNER] ARRAY JOIN list`.
pub(super) fn array_join_clause(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    if !p.at_kw(K::Array) {
        p.bump_keyword();
    }
    p.bump_keyword();
    p.bump_keyword();
    super::expr_list(p)?;
    m.complete(p, SyntaxKind::ArrayJoinClause);
    Some(())
}
