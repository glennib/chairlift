//! `FROM` and the tables in it (`ParserTablesInSelectQuery`).

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
    // `select`, which ClickHouse settles by parsing again; another `SELECT`
    // after it makes it an alias.
    let mode = if p.at_kw(K::Select) && !p.nth_at_kw(1, K::Select) {
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

/// The table itself: a subquery, a table function, a name, or a file name
/// as a string.
fn table(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    match p.nth(0) {
        Some(T::OpeningRoundBracket) => select::subquery(p),
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
    let no_in = std::mem::replace(&mut p.no_in, false);
    let args = if view == QueryArgument::None {
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
        select::query(p).and_then(|_| {
            if view == QueryArgument::QueryOrElse && p.eat_kw(K::Else) {
                let name = names::name(p)?;
                table_function(p, name, QueryArgument::None)?;
            }
            Some(())
        })
    };
    p.no_in = no_in;
    args?;
    p.expect(T::ClosingRoundBracket)?;
    m.complete(p, SyntaxKind::ArgList);
    Some(())
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
        exprs::expr(p)?;
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
