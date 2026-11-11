//! `SELECT` queries, after ClickHouse's `ParserSelectWithUnionQuery`,
//! `ParserSelectQuery`, `ParserQueryWithOutput` and
//! `ParserTablesInSelectQuery`.
//!
//! The functions here are also the seams the rest of the grammar calls:
//!
//! - [`at_statement`] and [`statement`] from the statement dispatch;
//! - [`at_subquery`] and [`subquery`] from expressions: `(SELECT …)`,
//!   `IN (SELECT …)`, `exists(SELECT …)`, `x = ANY (SELECT …)`;
//! - [`query`] wherever a query follows without parentheses of its own, such
//!   as `CREATE MATERIALIZED VIEW … AS SELECT …`;
//! - [`projection_subquery`] for `PROJECTION name (SELECT …)`;
//! - [`table_function`] for `CREATE TABLE … AS f(…)`, which takes the
//!   table functions `FROM` does.
//!
//! # Clauses
//!
//! A query is a run of clauses, each recognised by its leading keywords
//! ([`clause_at`]) and parsed inside [`fallback::clause`], so clauses may
//! come in any order and repeat, and one that does not parse is kept
//! verbatim up to the next clause keyword. That is what lets a template's
//! `{% if %} WHERE a {% else %} WHERE b {% endif %}` parse as two clauses.
//! ClickHouse reads its clauses in a fixed order; for valid queries both
//! readings give the same clauses.
//!
//! # Lists
//!
//! The lists of clauses ([`list`]) read an item after every comma, whatever
//! word it starts with, as ClickHouse's `ParserList` does: in `SELECT a,
//! limit FROM t` the second item is a column named `limit`. An item that
//! does not parse is kept verbatim up to the next comma or clause keyword.
//! A comma that ends a list (`SELECT a, FROM t`) is the list's last token.
//!
//! # Keyword slots
//!
//! Words that start a clause are names wherever ClickHouse reads a name:
//! `SELECT format FROM format FORMAT JSON` has one keyword `FORMAT`. The
//! positional rules, each as ClickHouse has it:
//!
//! - `SETTINGS` and `FORMAT` open a clause only before a name, `WINDOW` only
//!   before `name AS`, `FETCH` only before `FIRST` or `NEXT`, and a join's
//!   words only when `JOIN` follows them.
//! - An alias without `AS` cannot be one of the restricted words, so in
//!   `FROM t FINAL` the `FINAL` is a keyword; it can be `LOCAL`, so `FROM t
//!   LOCAL JOIN u` joins `t AS LOCAL`, as ClickHouse reads it. It can be
//!   `select` too, but for a table of the `FROM`-first form only where the
//!   tables go on after it (see [`tables::Aliases`]); the tables of joins in
//!   brackets take none.
//! - `ALL`, `DISTINCT` and `TOP n` right after `SELECT`, `ALL`, `ROLLUP(`,
//!   `CUBE(` and `GROUPING SETS (` right after `GROUP BY`, and `ALL` after
//!   `ORDER BY` unless a comma follows it, are keywords.

mod tables;

pub(crate) use tables::table_function_call as table_function;

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::exprs::AliasMode;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::names;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Whether the statement at the current token is a query: it starts with
/// `SELECT`, `WITH` or `(`, or with `FROM` and a table.
pub(crate) fn at_statement(p: &Parser<'_>) -> bool {
    p.at_kw(K::Select) || p.at_kw(K::With) || p.at(T::OpeningRoundBracket) || at_from_first(p, 0)
}

/// Whether the `n`th token starts the `FROM`-first form of a query, `FROM t
/// …`: `FROM` and a table, whatever word names it, so `FROM and SELECT 1`
/// selects from a table named `and`.
fn at_from_first(p: &Parser<'_>, n: usize) -> bool {
    p.nth_at_kw(n, K::From) && tables::at_table(p, n + 1)
}

/// [`at_from_first`] right after an opening bracket, where ClickHouse
/// (`parenthesesHoldExpressionOverColumnNamedFrom`) reads an expression over
/// a column named `from` if an operator follows it, so `(from BETWEEN 0 AND
/// 2)` is no subquery.
fn at_bracketed_from_first(p: &Parser<'_>, n: usize) -> bool {
    at_from_first(p, n) && !at_operator(p, n + 1)
}

/// Whether the `n`th token starts a binary or postfix operator
/// (`ParserExpressionImpl::operators_table`).
fn at_operator(p: &Parser<'_>, n: usize) -> bool {
    match p.nth(n) {
        Some(
            T::Arrow
            | T::QuestionMark
            | T::Colon
            | T::PipeMark
            | T::Equals
            | T::NotEquals
            | T::Spaceship
            | T::Less
            | T::Greater
            | T::LessOrEquals
            | T::GreaterOrEquals
            | T::Tilde
            | T::TildeAsterisk
            | T::NotTilde
            | T::NotTildeAsterisk
            | T::Concatenation
            | T::Plus
            | T::Minus
            | T::Asterisk
            | T::Slash
            | T::Percent
            | T::Dot
            | T::OpeningSquareBracket
            | T::DoubleColon,
        ) => true,
        Some(T::BareWord) => match p.nth_keyword(n) {
            Some(
                K::Or
                | K::And
                | K::Between
                | K::Like
                | K::Ilike
                | K::Regexp
                | K::In
                | K::Mod
                | K::Div,
            ) => true,
            Some(K::Is) => matches!(
                p.nth_keyword(n + 1),
                Some(K::Not | K::Distinct | K::Unknown | K::False | K::True | K::Null)
            ),
            Some(K::Not) => matches!(
                p.nth_keyword(n + 1),
                Some(K::Between | K::Like | K::Ilike | K::In | K::Similar)
            ),
            Some(K::Global) => matches!(p.nth_keyword(n + 1), Some(K::In | K::Not)),
            Some(K::Similar) => p.nth_at_kw(n + 1, K::To),
            Some(K::At) => p.nth_at_kw(n + 1, K::Time) && p.nth_at_kw(n + 2, K::Zone),
            _ => false,
        },
        _ => false,
    }
}

/// A query statement, up to the end of the statement.
///
/// ClickHouse's pipe syntax (`FROM t |> WHERE x |> SELECT y`) has an
/// operator grammar of its own, which is not parsed: a statement with `|>`
/// fails as a whole. Parsing around it would end the clause before `|>`
/// verbatim at the next clause keyword, which in `FROM select s |> …` is
/// the table's name.
pub(crate) fn statement(p: &mut Parser<'_>) -> Option<()> {
    if (0..)
        .map_while(|n| p.nth(n))
        .any(|kind| kind == T::PipeOperator)
    {
        return p.expected("a query without `|>`");
    }
    query(p, Level::Statement).map(drop)
}

/// Where a query stands, which says whether output clauses follow it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    /// A statement of its own, which `INTO OUTFILE`, `FORMAT` and a
    /// `SETTINGS` after them may follow (`ParserQueryWithOutput`).
    Statement,
    /// Part of something else: a subquery, a table function's argument, or
    /// the query of `CREATE … AS`, whose output clauses, if any, are the
    /// `CREATE` statement's.
    Nested,
}

/// A query, up to the end of the statement or a closing bracket that does
/// not belong to it: a [`SelectQuery`](SyntaxKind::SelectQuery), or a
/// [`UnionQuery`](SyntaxKind::UnionQuery) for set operations and queries in
/// parentheses. It also ends before `COMMENT 'text'`, which follows the
/// query of `CREATE VIEW … AS SELECT …`.
///
/// At the [statement level](Level::Statement), the output clauses follow
/// as the query's own children, after its operands: ClickHouse attaches
/// them to the whole set operation, wherever the brackets are. A `SETTINGS`
/// right after an operand without brackets is that operand's, as ClickHouse
/// reads it; one after brackets or another output clause is the
/// statement's.
pub(crate) fn query(p: &mut Parser<'_>, level: Level) -> Option<CompletedMarker> {
    p.nested(|p| {
        let first = operand(p)?;
        let (m, kind) = if at_set_operator(p) {
            let m = first.complete(p).precede(p);
            while at_set_operator(p) {
                let op = p.start();
                p.bump_keyword();
                if p.at_kw(K::All) || p.at_kw(K::Distinct) {
                    p.bump_keyword();
                }
                op.complete(p, SyntaxKind::SetOperator);
                operand(p)?.complete(p);
            }
            (m, SyntaxKind::UnionQuery)
        } else {
            match first {
                Operand::Select(m) => (m, SyntaxKind::SelectQuery),
                Operand::Subquery(subquery) => (subquery.precede(p), SyntaxKind::UnionQuery),
            }
        };
        if level == Level::Statement {
            output_clauses(p);
        }
        Some(m.complete(p, kind))
    })
}

/// An operand of a set operation: a query in parentheses, or the clauses of
/// one without, whose [`SelectQuery`](SyntaxKind::SelectQuery) node is left
/// open for the output clauses of a statement that is only that query.
enum Operand {
    Subquery(CompletedMarker),
    Select(crate::syntax::parser::Marker),
}

impl Operand {
    fn complete(self, p: &mut Parser<'_>) -> CompletedMarker {
        match self {
            Operand::Subquery(subquery) => subquery,
            Operand::Select(m) => m.complete(p, SyntaxKind::SelectQuery),
        }
    }
}

fn operand(p: &mut Parser<'_>) -> Option<Operand> {
    if p.at(T::OpeningRoundBracket) {
        subquery(p).map(Operand::Subquery)
    } else {
        select_query(p).map(Operand::Select)
    }
}

/// The output clauses of a statement (`ParserQueryWithOutput`): `INTO
/// OUTFILE`, `FORMAT` and `SETTINGS`, each with the clause fallback, in any
/// order and repeated, as clauses are.
fn output_clauses(p: &mut Parser<'_>) {
    loop {
        let parse = if common::at_into_outfile(p) {
            common::into_outfile_clause
        } else if common::at_format(p) {
            common::format_clause
        } else if common::at_settings(p) {
            |p: &mut Parser<'_>| common::settings_clause(p, &at_clause)
        } else {
            return;
        };
        fallback::clause(p, &at_clause, |p| parse(p).map(drop));
    }
}

fn at_set_operator(p: &Parser<'_>) -> bool {
    matches!(p.nth_keyword(0), Some(K::Union | K::Except | K::Intersect))
}

/// Whether the `n`th token opens a subquery: `(` followed by `SELECT`,
/// `WITH` or `EXPLAIN` and something a query can go on with, by `VALUES
/// (`, or by `FROM` and a table (`(FROM t)`, which `(from + 1)`, over a
/// column named `from`, is not).
///
/// ClickHouse tries a subquery first and reads an expression if that fails,
/// so `(select)`, `(select, y)` and `(select = 1)` are over a column named
/// `select`; a word after `SELECT` starts its list, so `(select and x)` is
/// `(SELECT and AS x)`. A query needs `SELECT` or `FROM` after its `WITH`
/// clause, so `(with - 1)` and `(with [1])` are over a column named `with`.
pub(crate) fn at_subquery(p: &Parser<'_>, n: usize) -> bool {
    p.nth_at(n, T::OpeningRoundBracket)
        && match p.nth_keyword(n + 1) {
            Some(keyword @ (K::Select | K::With | K::Explain)) => {
                !matches!(p.nth(n + 2), None | Some(T::Comma | T::ClosingRoundBracket))
                    && !at_symbol_operator(p, n + 2)
                    && (keyword != K::With || select_follows_with(p, n))
            }
            Some(K::Values) => p.nth_at(n + 2, T::OpeningRoundBracket),
            Some(K::From) => at_bracketed_from_first(p, n + 1),
            _ => false,
        }
}

/// Whether the bracket group that opens at the `n`th token, and holds a
/// `WITH` clause, is closed and has `SELECT` or `FROM` among the tokens
/// directly inside it, which a query needs after `WITH`.
fn select_follows_with(p: &Parser<'_>, n: usize) -> bool {
    p.closer(n).is_some()
        && p.group_items(n)
            .skip(1)
            .any(|i| matches!(p.nth_keyword(i), Some(K::Select | K::From)))
}

/// Whether the `n`th token is an operator that no expression starts with:
/// [`at_operator`]'s symbols but `-`, `+`, `*` and `[`.
fn at_symbol_operator(p: &Parser<'_>, n: usize) -> bool {
    !matches!(
        p.nth(n),
        Some(T::BareWord | T::Minus | T::Plus | T::Asterisk | T::OpeningSquareBracket)
    ) && at_operator(p, n)
}

/// A [`Subquery`](SyntaxKind::Subquery): `(`, a [`query`] or the rows of
/// `VALUES`, `)`. An `EXPLAIN` in parentheses is kept as written: it is a
/// statement of its own, which the grammar keeps verbatim elsewhere too.
pub(crate) fn subquery(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect(T::OpeningRoundBracket)?;
    p.with_no_in(false, |p| {
        p.with_else_ending_query(false, |p| {
            if p.at_kw(K::Explain) {
                fallback::opaque_until(p, &fallback::never)
            } else if p.at_kw(K::Values) && p.nth_at(1, T::OpeningRoundBracket) {
                let values = p.start();
                p.bump_keyword();
                list(p, SyntaxKind::ExprList, &mut |p| exprs::expr(p).map(drop))
                    .map(|_| values.complete(p, SyntaxKind::ValuesRows))
            } else {
                query(p, Level::Nested)
            }
        })
    })?;
    p.expect(T::ClosingRoundBracket)?;
    Some(m.complete(p, SyntaxKind::Subquery))
}

/// Whether the current token starts a query without parentheses.
fn at_query(p: &Parser<'_>) -> bool {
    p.at_kw(K::Select) || p.at_kw(K::With) || at_from_first(p, 0)
}

/// The clauses of a [`SelectQuery`](SyntaxKind::SelectQuery), which must
/// include `SELECT` or `FROM`, and the query's node, open.
fn select_query(p: &mut Parser<'_>) -> Option<crate::syntax::parser::Marker> {
    if !at_query(p) {
        return p.expected("a query");
    }
    let m = p.start();
    if !clauses(p, Form::Query) {
        m.abandon(p);
        return p.expected("SELECT");
    }
    Some(m)
}

/// The query of `PROJECTION name (…)`, as a [`Subquery`](SyntaxKind::Subquery)
/// holding a [`SelectQuery`](SyntaxKind::SelectQuery).
///
/// ClickHouse reads it with `ParserProjectionSelectQuery`: `[WITH list]
/// SELECT list [WHERE e] [GROUP BY list] [ORDER BY list]`, without the
/// modifiers of a query, so `distinct`, `all` and `top` after `SELECT`, and
/// `all`, `rollup(…)` and `cube` after `GROUP BY` or `ORDER BY`, are names
/// there, and `WITH` takes no CTEs.
pub(crate) fn projection_subquery(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect(T::OpeningRoundBracket)?;
    if !(p.at_kw(K::Select) || p.at_kw(K::With)) {
        return p.expected("SELECT");
    }
    p.with_no_in(false, |p| {
        let query = p.start();
        if !clauses(p, Form::Projection) {
            query.abandon(p);
            return p.expected("SELECT");
        }
        query.complete(p, SyntaxKind::SelectQuery);
        Some(())
    })?;
    p.expect(T::ClosingRoundBracket)?;
    Some(m.complete(p, SyntaxKind::Subquery))
}

/// Which grammar a query's clauses follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    Query,
    /// A projection's query, see [`projection_subquery`].
    Projection,
}

// ---- Clauses ---------------------------------------------------------------

/// The clauses of a query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Clause {
    With,
    /// `WITH TOTALS`, `WITH ROLLUP`, `WITH CUBE` without `GROUP BY`.
    GroupByModifiers,
    Select,
    From,
    Join,
    ArrayJoin,
    Prewhere,
    Where,
    GroupBy,
    Having,
    Window,
    Qualify,
    OrderBy,
    Interpolate,
    Limit,
    Offset,
    Fetch,
    Settings,
}

/// The clause that starts at the current token, if any.
fn clause_at(p: &Parser<'_>) -> Option<Clause> {
    Some(match p.nth_keyword(0) {
        Some(K::With) if at_group_by_modifier(p) => Clause::GroupByModifiers,
        Some(K::With) => Clause::With,
        Some(K::Select) => Clause::Select,
        Some(K::From) => Clause::From,
        Some(K::Prewhere) => Clause::Prewhere,
        Some(K::Where) => Clause::Where,
        Some(K::Group) if p.nth_at_kw(1, K::By) => Clause::GroupBy,
        Some(K::Having) => Clause::Having,
        Some(K::Window) if names::at_name(p, 1) && p.nth_at_kw(2, K::As) => Clause::Window,
        Some(K::Qualify) => Clause::Qualify,
        Some(K::Order) if p.nth_at_kw(1, K::By) => Clause::OrderBy,
        Some(K::Interpolate) => Clause::Interpolate,
        Some(K::Limit) => Clause::Limit,
        Some(K::Offset) => Clause::Offset,
        Some(K::Fetch) if matches!(p.nth_keyword(1), Some(K::First | K::Next)) => Clause::Fetch,
        Some(K::Settings) if common::at_settings(p) => Clause::Settings,
        _ if tables::at_array_join(p, 0) => Clause::ArrayJoin,
        _ if tables::at_join(p) => Clause::Join,
        _ => return None,
    })
}

/// Whether a query's clause, list or verbatim region ends here: at a clause
/// keyword, an output clause's (`FORMAT`, `INTO OUTFILE`), a set operator,
/// `COMMENT 'text'`, which ends the query of a `CREATE VIEW`, or `ELSE`
/// where it ends the query of `viewIfPermitted(SELECT … ELSE f())`.
fn at_clause(p: &Parser<'_>) -> bool {
    clause_at(p).is_some()
        || common::at_format(p)
        || common::at_into_outfile(p)
        || at_set_operator(p)
        || (p.at_kw(K::Comment) && p.nth_at(1, T::StringLiteral))
        || (p.else_ends_query() && p.at_kw(K::Else))
}

/// [`at_clause`], or a comma, which may follow a join to join another table.
fn at_clause_or_comma(p: &Parser<'_>) -> bool {
    at_clause(p) || p.at(T::Comma)
}

/// The clauses from the current token, each with [`fallback::clause`], up
/// to anything that starts no clause. Returns whether they include `SELECT`,
/// or in a query `FROM`, parsed or not: ClickHouse reads no query without
/// one, so a `WITH` clause alone is none.
fn clauses(p: &mut Parser<'_>, form: Form) -> bool {
    let mut after_join = false;
    let mut first = true;
    let mut selected = false;
    let mut selects = false;
    loop {
        let mut clause = clause_at(p);
        // `WITH totals SELECT …` starts with a `WITH` clause.
        if first && clause == Some(Clause::GroupByModifiers) {
            clause = Some(Clause::With);
        }
        first = false;
        let comma_join = clause.is_none() && after_join && p.at(T::Comma);
        if clause.is_none() && !comma_join {
            return selects;
        }
        selects |=
            clause == Some(Clause::Select) || (form == Form::Query && clause == Some(Clause::From));
        // Tables before the `SELECT` clause are the `FROM`-first form's.
        let aliases = if selected {
            tables::Aliases::Implicit
        } else {
            tables::Aliases::FromFirst
        };
        selected |= clause == Some(Clause::Select);
        let join = comma_join || matches!(clause, Some(Clause::Join));
        // A clause that does not parse is verbatim up to the next clause,
        // which cannot start among its own keywords: the `JOIN` of `LEFT
        // JOIN`, or the `ARRAY JOIN` of `LEFT ARRAY JOIN`.
        let after_keywords = p.position() + clause.map_or(1, |c| keyword_count(p, c));
        let ends = |p: &Parser<'_>| {
            p.position() >= after_keywords
                && if join {
                    at_clause_or_comma(p)
                } else {
                    at_clause(p)
                }
        };
        fallback::clause(p, &ends, |p| match clause {
            None => tables::comma_join(p, aliases),
            Some(clause) => parse_clause(p, clause, form, aliases),
        });
        after_join = join;
    }
}

/// How many keywords start `clause` at the current token.
fn keyword_count(p: &Parser<'_>, clause: Clause) -> usize {
    match clause {
        Clause::GroupByModifiers | Clause::GroupBy | Clause::OrderBy | Clause::Fetch => 2,
        Clause::ArrayJoin => 2 + usize::from(!p.at_kw(K::Array)),
        Clause::Join => tables::join_keywords(p, 0).unwrap_or(1),
        _ => 1,
    }
}

/// The clause `clause` at the current token, whose tables take aliases as
/// `aliases` allows.
fn parse_clause(
    p: &mut Parser<'_>,
    clause: Clause,
    form: Form,
    aliases: tables::Aliases,
) -> Option<()> {
    match clause {
        Clause::With => with_clause(p, form),
        Clause::Select => select_clause(p, form),
        Clause::From => tables::from_clause(p, aliases),
        Clause::Join => tables::join_clause(p, aliases),
        Clause::ArrayJoin => tables::array_join_clause(p),
        Clause::Prewhere => expression_clause(p, SyntaxKind::PrewhereClause),
        Clause::Where => expression_clause(p, SyntaxKind::WhereClause),
        Clause::GroupBy | Clause::GroupByModifiers => group_by_clause(p, form),
        Clause::Having => expression_clause(p, SyntaxKind::HavingClause),
        Clause::Window => window_clause(p),
        Clause::Qualify => expression_clause(p, SyntaxKind::QualifyClause),
        Clause::OrderBy => order_by_clause(p, form),
        Clause::Interpolate => interpolate_clause(p),
        Clause::Limit => limit_clause(p),
        Clause::Offset => offset_clause(p),
        Clause::Fetch => fetch_clause(p),
        Clause::Settings => common::settings_clause(p, &at_clause).map(drop),
    }
}

/// Items separated by commas as a node of `kind`, each parsed by `item` with
/// [`fallback::list_item`]'s fallback, ending at a clause keyword: an item is
/// read after every comma (see the [module documentation](self#lists)).
/// Fails if there is no item.
fn list(
    p: &mut Parser<'_>,
    kind: SyntaxKind,
    item: &mut dyn FnMut(&mut Parser<'_>) -> Option<()>,
) -> Option<CompletedMarker> {
    list_with(p, kind, &fallback::never, item)
}

/// [`list`], which also ends after a comma where `trailing` says the comma
/// is the list's last token.
fn list_with(
    p: &mut Parser<'_>,
    kind: SyntaxKind,
    trailing: fallback::Stop<'_>,
    item: &mut dyn FnMut(&mut Parser<'_>) -> Option<()>,
) -> Option<CompletedMarker> {
    let m = p.start();
    let mut items = 0usize;
    while !(p.at_end() || p.at_closer() || (items > 0 && trailing(p))) {
        let before = p.position();
        fallback::list_item(p, &at_clause, &mut *item);
        if p.position() == before {
            break;
        }
        items += 1;
        if !p.eat(T::Comma) {
            break;
        }
    }
    if items == 0 {
        m.abandon(p);
        return p.expected("a list item");
    }
    Some(m.complete(p, kind))
}

/// An [`ExprList`](SyntaxKind::ExprList) of a clause, whose items may have
/// an `AS` alias.
fn expr_list(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    list(p, SyntaxKind::ExprList, &mut |p| {
        exprs::expr_with_alias(p, AliasMode::As).map(drop)
    })
}

/// A [`ParenList`](SyntaxKind::ParenList) of `item`s.
fn paren_list(
    p: &mut Parser<'_>,
    item: &mut dyn FnMut(&mut Parser<'_>) -> Option<()>,
) -> Option<CompletedMarker> {
    fallback::delimited(
        p,
        SyntaxKind::ParenList,
        T::OpeningRoundBracket,
        T::ClosingRoundBracket,
        item,
    )
}

/// A [`ParenList`](SyntaxKind::ParenList) of names.
fn name_list(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    paren_list(p, &mut |p| names::name(p).map(drop))
}

/// A [`ParenList`](SyntaxKind::ParenList) of expressions with any `AS`
/// alias.
fn paren_expr_list(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    paren_list(p, &mut |p| {
        exprs::expr_with_alias(p, AliasMode::As).map(drop)
    })
}

/// A clause of a keyword and an expression, which may have an `AS` alias
/// (`WHERE (a = 1) AS cond`, which ClickHouse accepts).
fn expression_clause(p: &mut Parser<'_>, kind: SyntaxKind) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    exprs::expr_with_alias(p, AliasMode::As)?;
    m.complete(p, kind);
    Some(())
}

/// `WITH [RECURSIVE] list`; in a projection only `WITH list` of
/// expressions.
fn with_clause(p: &mut Parser<'_>, form: Form) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    if form == Form::Query {
        p.eat_kw(K::Recursive);
    }
    list(p, SyntaxKind::WithList, &mut |p| {
        if form == Form::Query && at_common_table_expr(p) {
            common_table_expr(p)
        } else {
            exprs::expr_with_alias(p, AliasMode::As).map(drop)
        }
    })?;
    m.complete(p, SyntaxKind::WithClause);
    Some(())
}

/// Whether a CTE starts here (`ParserWithElement`): a name other than a
/// bare `select`, any column names in parentheses, `AS`, any
/// `MATERIALIZED`, and `(`, which no alias can be. Otherwise the item is an
/// expression with an alias, `expr AS name`.
fn at_common_table_expr(p: &Parser<'_>) -> bool {
    if !matches!(
        p.nth(0),
        Some(T::BareWord | T::QuotedIdentifier | T::JinjaExpr)
    ) || p.at_kw(K::Select)
    {
        return false;
    }
    let mut n = 1;
    if p.nth_at(n, T::OpeningRoundBracket) {
        n += 1;
        while names::at_name(p, n) {
            n += 1;
            if !p.nth_at(n, T::Comma) {
                break;
            }
            n += 1;
        }
        if !p.nth_at(n, T::ClosingRoundBracket) {
            return false;
        }
        n += 1;
    }
    if !p.nth_at_kw(n, K::As) {
        return false;
    }
    p.nth_at(n + 1, T::OpeningRoundBracket)
        || (p.nth_at_kw(n + 1, K::Materialized) && p.nth_at(n + 2, T::OpeningRoundBracket))
}

fn common_table_expr(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    names::name(p)?;
    if p.at(T::OpeningRoundBracket) {
        name_list(p)?;
    }
    p.expect_kw(K::As)?;
    p.eat_kw(K::Materialized);
    subquery(p)?;
    m.complete(p, SyntaxKind::CommonTableExpr);
    Some(())
}

/// `SELECT [ALL] [DISTINCT [ON (…)]] [ALL] [TOP n [WITH TIES]] list`; in a
/// projection `SELECT list`.
fn select_clause(p: &mut Parser<'_>, form: Form) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    if form == Form::Projection {
        list(p, SyntaxKind::SelectList, &mut |p| {
            exprs::expr_with_alias(p, AliasMode::Implicit).map(drop)
        })?;
        m.complete(p, SyntaxKind::SelectClause);
        return Some(());
    }
    let all = p.eat_kw(K::All);
    if p.at_kws(&[K::Distinct, K::On]) && p.nth_at(2, T::OpeningRoundBracket) {
        let d = p.start();
        p.bump_keyword();
        p.bump_keyword();
        paren_expr_list(p)?;
        d.complete(p, SyntaxKind::DistinctOn);
    } else {
        p.eat_kw(K::Distinct);
    }
    if !all {
        p.eat_kw(K::All);
    }
    if at_top(p) {
        let t = p.start();
        p.bump_keyword();
        if p.eat(T::OpeningRoundBracket) {
            exprs::literal(p)?;
            p.expect(T::ClosingRoundBracket)?;
        } else {
            exprs::literal(p)?;
        }
        p.eat_kws(&[K::With, K::Ties]);
        t.complete(p, SyntaxKind::TopClause);
    }
    list_with(
        p,
        SyntaxKind::SelectList,
        &at_from_after_trailing_comma,
        &mut |p| exprs::expr_with_alias(p, AliasMode::Implicit).map(drop),
    )?;
    m.complete(p, SyntaxKind::SelectClause);
    Some(())
}

/// Whether `FROM` after a comma of a `SELECT` list starts the `FROM` clause,
/// so that the comma is a trailing one: unless what follows `from` makes it
/// a column, as in `SELECT to, from FROM t` or `SELECT from + 1`
/// (`ExpressionLayer::parse` with `allow_trailing_commas`).
fn at_from_after_trailing_comma(p: &Parser<'_>) -> bool {
    let column = p.nth_at(1, T::Comma)
        || p.nth_at_kw(1, K::From)
        || (p.nth_at_kw(1, K::As) && names::at_name(p, 2))
        || at_operator(p, 1);
    p.at_kw(K::From) && !column
}

/// Whether `TOP n` or `TOP (n)` follows, with a number for `n` as
/// ClickHouse requires.
fn at_top(p: &Parser<'_>) -> bool {
    p.at_kw(K::Top)
        && (p.nth_at(1, T::Number)
            || (p.nth_at(1, T::OpeningRoundBracket)
                && p.nth_at(2, T::Number)
                && p.nth_at(3, T::ClosingRoundBracket)))
}

/// Whether `WITH ROLLUP`, `WITH CUBE` or `WITH TOTALS` follows.
fn at_group_by_modifier(p: &Parser<'_>) -> bool {
    p.at_kw(K::With) && matches!(p.nth_keyword(1), Some(K::Rollup | K::Cube | K::Totals))
}

/// `GROUP BY …` and its modifiers, or the modifiers alone; in a projection
/// `GROUP BY list`.
fn group_by_clause(p: &mut Parser<'_>, form: Form) -> Option<()> {
    let m = p.start();
    if p.at_kw(K::Group) {
        p.bump_keyword();
        p.bump_keyword();
        if form == Form::Projection {
            expr_list(p)?;
        } else if (p.at_kw(K::Rollup) || p.at_kw(K::Cube)) && p.nth_at(1, T::OpeningRoundBracket) {
            p.bump_keyword();
            paren_expr_list(p)?;
        } else if p.at_kws(&[K::Grouping, K::Sets]) && p.nth_at(2, T::OpeningRoundBracket) {
            grouping_sets(p)?;
        } else if !p.eat_kw(K::All) {
            expr_list(p)?;
        }
    }
    while at_group_by_modifier(p) {
        let modifier = p.start();
        p.bump_keyword();
        p.bump_keyword();
        modifier.complete(p, SyntaxKind::GroupByModifier);
    }
    m.complete(p, SyntaxKind::GroupByClause);
    Some(())
}

/// `GROUPING SETS ((a, b), c, ())`: an item in parentheses is a set of
/// expressions, anything else one expression.
fn grouping_sets(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.bump_keyword();
    p.expect(T::OpeningRoundBracket)?;
    fallback::comma_separated(p, &fallback::never, &mut |p| {
        if p.at(T::OpeningRoundBracket) {
            paren_expr_list(p).map(drop)
        } else {
            exprs::expr_with_alias(p, AliasMode::As).map(drop)
        }
    });
    p.expect(T::ClosingRoundBracket)?;
    m.complete(p, SyntaxKind::GroupingSets);
    Some(())
}

/// `WINDOW name AS (…), …`.
fn window_clause(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    list(p, SyntaxKind::WindowList, &mut |p| {
        let w = p.start();
        names::name(p)?;
        p.expect_kw(K::As)?;
        fallback::opaque_group(p)?;
        w.complete(p, SyntaxKind::WindowDefinition);
        Some(())
    })?;
    m.complete(p, SyntaxKind::WindowClause);
    Some(())
}

/// `ORDER BY list`, or, but in a projection, `ORDER BY ALL [ASC|DESC]
/// [NULLS FIRST|LAST]`.
fn order_by_clause(p: &mut Parser<'_>, form: Form) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.bump_keyword();
    if let Some(n) = order_by_all(p).filter(|_| form == Form::Query) {
        for _ in 0..n {
            p.bump_keyword();
        }
    } else {
        list(p, SyntaxKind::OrderByList, &mut |p| {
            common::order_by_item(p).map(drop)
        })?;
    }
    m.complete(p, SyntaxKind::OrderByClause);
    Some(())
}

/// How many tokens `ALL [ASC|DESC] [NULLS FIRST|LAST]` takes, if that
/// follows and no comma after it makes `all` a column.
fn order_by_all(p: &Parser<'_>) -> Option<usize> {
    if !p.at_kw(K::All) {
        return None;
    }
    let mut n = 1;
    if matches!(
        p.nth_keyword(n),
        Some(K::Asc | K::Desc | K::Ascending | K::Descending)
    ) {
        n += 1;
    }
    if p.nth_at_kw(n, K::Nulls) && matches!(p.nth_keyword(n + 1), Some(K::First | K::Last)) {
        n += 2;
    }
    (!p.nth_at(n, T::Comma)).then_some(n)
}

/// `INTERPOLATE [(column [AS expr], …)]`.
fn interpolate_clause(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    if p.at(T::OpeningRoundBracket) {
        paren_list(p, &mut |p| {
            let item = p.start();
            names::name(p)?;
            if p.eat_kw(K::As) {
                exprs::expr(p)?;
            }
            item.complete(p, SyntaxKind::InterpolateItem);
            Some(())
        })?;
    }
    m.complete(p, SyntaxKind::InterpolateClause);
    Some(())
}

/// `LIMIT …`: a [`LimitByClause`](SyntaxKind::LimitByClause) if `BY`
/// follows the counts, a [`LimitClause`](SyntaxKind::LimitClause)
/// otherwise.
fn limit_clause(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    if limit_range(p)? {
        m.complete(p, SyntaxKind::LimitClause);
        return Some(());
    }
    limit_expr(p)?;
    if p.eat(T::Comma) || p.eat_kw(K::Offset) {
        limit_expr(p)?;
    }
    p.eat_kws(&[K::With, K::Ties]);
    if p.eat_kw(K::By) {
        if !p.eat_kw(K::All) {
            expr_list(p)?;
        }
        m.complete(p, SyntaxKind::LimitByClause);
        return Some(());
    }
    limit_range(p)?;
    m.complete(p, SyntaxKind::LimitClause);
    Some(())
}

fn limit_expr(p: &mut Parser<'_>) -> Option<()> {
    exprs::expr_with_alias(p, AliasMode::As).map(drop)
}

/// `AFTER a [ALL] [UNTIL b]` or `UNTIL b` (`parseLimitRange`), if it
/// follows: `Some(true)` if it parsed, `Some(false)` if it does not follow.
/// The words are keywords only before an expression, so `LIMIT after BY x`
/// counts by a column named `after`.
fn limit_range(p: &mut Parser<'_>) -> Option<bool> {
    let after = p.at_kw(K::After);
    if !(after || p.at_kw(K::Until))
        || matches!(
            p.nth(1),
            None | Some(T::Comma | T::Semicolon | T::ClosingRoundBracket)
        )
        || at_limit_count_follower(p, 1)
    {
        return Some(false);
    }
    p.bump_keyword();
    limit_expr(p)?;
    if after {
        p.eat_kw(K::All);
        if p.eat_kw(K::Until) {
            limit_expr(p)?;
        }
    }
    Some(true)
}

/// Whether the `n`th token is a word that may follow a `LIMIT` count.
fn at_limit_count_follower(p: &Parser<'_>, n: usize) -> bool {
    match p.nth_keyword(n) {
        Some(
            K::As
            | K::By
            | K::Offset
            | K::After
            | K::Until
            | K::Fetch
            | K::Settings
            | K::Format
            | K::Union
            | K::Except
            | K::Intersect,
        ) => true,
        Some(K::With) => p.nth_at_kw(n + 1, K::Ties),
        Some(K::Into) => p.nth_at_kw(n + 1, K::Outfile),
        Some(K::Parallel) => p.nth_at_kw(n + 1, K::With),
        _ => false,
    }
}

/// `OFFSET n [ROW | ROWS]`.
fn offset_clause(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    limit_expr(p)?;
    if p.at_kw(K::Row) || p.at_kw(K::Rows) {
        p.bump_keyword();
    }
    m.complete(p, SyntaxKind::OffsetClause);
    Some(())
}

/// `FETCH FIRST|NEXT n ROW|ROWS ONLY|WITH TIES`.
fn fetch_clause(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.bump_keyword();
    limit_expr(p)?;
    if !(p.eat_kw(K::Row) || p.eat_kw(K::Rows)) {
        return p.expected("ROWS");
    }
    if !(p.eat_kw(K::Only) || p.eat_kws(&[K::With, K::Ties])) {
        return p.expected("ONLY");
    }
    m.complete(p, SyntaxKind::FetchClause);
    Some(())
}

#[cfg(test)]
mod tests;
