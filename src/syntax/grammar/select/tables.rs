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

/// `FROM` and a [`TableList`](SyntaxKind::TableList).
pub(super) fn from_clause(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    table_list(p)?;
    m.complete(p, SyntaxKind::FromClause);
    Some(())
}

fn table_list(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    super::list(p, SyntaxKind::TableList, &mut |p| table_expr(p).map(drop))
}

/// A [`TableExpr`](SyntaxKind::TableExpr) (`ParserTableExpression`).
fn table_expr(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let table = table(p)?;
    // `FROM t SELECT x` is the `FROM`-first form, not `t` with the alias
    // `select`, which ClickHouse settles by parsing again; a join's
    // condition or another `SELECT` after it makes it an alias.
    let mode =
        if p.at_kw(K::Select) && !matches!(p.nth_keyword(1), Some(K::On | K::Using | K::Select)) {
            AliasMode::As
        } else {
            AliasMode::Implicit
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
    Some(m.complete(p, SyntaxKind::TableExpr))
}

/// The table itself: a subquery, joins in parentheses, a table function, a
/// name, or a file name as a string.
fn table(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    match p.nth(0) {
        Some(T::OpeningRoundBracket) if at_parenthesized_query(p) => select::subquery(p),
        Some(T::OpeningRoundBracket) => p.nested(paren_join),
        Some(T::StringLiteral) => exprs::literal(p),
        _ => {
            let view = view_at(p, 0);
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
/// (SELECT 2))`. Otherwise it holds joins. Only the first few brackets of a
/// run are looked at, so that nested brackets cost linear time.
fn at_parenthesized_query(p: &Parser<'_>) -> bool {
    (0..32)
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
}

/// What `view(`, `eval(` and `viewIfPermitted(` at the `n`th token take:
/// a query if one follows the bracket.
fn view_at(p: &Parser<'_>, n: usize) -> QueryArgument {
    if !(p.nth_at(n, T::BareWord) && p.nth_at(n + 1, T::OpeningRoundBracket) && at_query(p, n + 2))
    {
        return QueryArgument::None;
    }
    let name = p.nth_text(n);
    if ["view", "eval", "obfuscate"]
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
    p.nth_at_kw(n, K::Select) || p.nth_at_kw(n, K::With) || select::at_from_first(p, n)
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
    p.nested(|p| table_function_args(p, view))?;
    Some(m.complete(p, SyntaxKind::FunctionCall))
}

/// The arguments of a table function: a query for `view` and its relatives,
/// otherwise expressions, a nested `view(SELECT …)`, or `SETTINGS a = 1, …`,
/// which takes the rest of the list.
fn table_function_args(p: &mut Parser<'_>, view: QueryArgument) -> Option<()> {
    let m = p.start();
    p.expect(T::OpeningRoundBracket)?;
    p.with_no_in(false, |p| table_function_body(p, view))?;
    p.expect(T::ClosingRoundBracket)?;
    m.complete(p, SyntaxKind::ArgList);
    Some(())
}

/// What lies between the brackets of a table function's arguments.
fn table_function_body(p: &mut Parser<'_>, view: QueryArgument) -> Option<()> {
    if view == QueryArgument::None {
        fallback::comma_separated(p, &fallback::never, &mut |p| {
            let nested = view_at(p, 0);
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
        select::query(p)?;
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
        table_expr(p)?;
        count += 1;
        if !p.eat(T::Comma) {
            break;
        }
    }
    tables.complete(p, SyntaxKind::TableList);
    let mut joined = count > 1;
    let mut after_join = false;
    loop {
        if at_array_join(p) {
            array_join_clause(p)?;
        } else if at_join(p) {
            join_clause(p)?;
        } else if after_join && p.at(T::Comma) {
            comma_join(p)?;
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
/// at the current token: `[GLOBAL|LOCAL] [NATURAL] [ANY|ALL|ASOF|SEMI|ANTI|
/// ONLY] [INNER|LEFT|RIGHT|FULL|CROSS|PASTE] [ANY|…] [OUTER] JOIN`.
pub(super) fn join_keywords(p: &Parser<'_>) -> Option<usize> {
    let strictness = |k| {
        matches!(
            k,
            Some(K::Any | K::All | K::Asof | K::Semi | K::Anti | K::Only)
        )
    };
    let mut n = 0;
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
    p.nth_at_kw(n, K::Join).then_some(n + 1)
}

/// Whether a join starts at the current token.
pub(super) fn at_join(p: &Parser<'_>) -> bool {
    join_keywords(p).is_some()
}

/// A [`JoinClause`](SyntaxKind::JoinClause): its keywords, any `LATERAL`, a
/// table, and an `ON` or `USING` condition if one follows.
pub(super) fn join_clause(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    for _ in 0..join_keywords(p)? {
        p.bump_keyword();
    }
    // `LATERAL` is a keyword only before a subquery; `JOIN lateral ON …`
    // joins a table named `lateral`.
    if p.at_kw(K::Lateral) && select::at_subquery(p, 1) {
        p.bump_keyword();
    }
    table_expr(p)?;
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
pub(super) fn comma_join(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.expect(T::Comma)?;
    table_expr(p)?;
    m.complete(p, SyntaxKind::JoinClause);
    Some(())
}

/// Whether `[LEFT | INNER] ARRAY JOIN` starts at the current token.
pub(super) fn at_array_join(p: &Parser<'_>) -> bool {
    let n = usize::from(p.at_kw(K::Left) || p.at_kw(K::Inner));
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
