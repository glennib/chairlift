//! Expressions: a Pratt parser with ClickHouse's operator priorities
//! (`operators_table` in `src/Parsers/ExpressionListParsers.cpp`).
//!
//! | Priority | Operators | Shape |
//! | --- | --- | --- |
//! | 14 | `.` `[…]` `::` | postfix, left to right |
//! | 13 | unary `-`, `AT TIME ZONE`, `AT LOCAL` | |
//! | 12 | `*` `/` `%` `MOD` `DIV` | left-associative |
//! | 11 | `+` `-` | |
//! | 10 | `\|\|` | |
//! | 9 | `=` `!=` `<` `>` `<=` `>=` `<=>` `~` `LIKE` `ILIKE` `REGEXP` `SIMILAR TO` `IN` `GLOBAL IN` and their `NOT` forms | |
//! | 8 | `\|` | |
//! | 7 | `[NOT] BETWEEN … AND …` | lower bound takes 8 and up, upper 9 and up |
//! | 6 | `IS [NOT] NULL\|TRUE\|FALSE\|UNKNOWN`, `IS [NOT] DISTINCT FROM` | |
//! | 5 | `NOT` | prefix |
//! | 4 | `AND` | one [`AndExpr`](SyntaxKind::AndExpr) per run |
//! | 3 | `OR` | one [`OrExpr`](SyntaxKind::OrExpr) per run |
//! | 2, 3 | `? :` | |
//! | 1 | `->` | |
//!
//! Binding powers are twice the priority. Operands that ClickHouse reads
//! before any operator follow its order too: a negative number is one
//! operand (`-1::Int8` casts `-1`), `NULL`, `true`, `inf` and `nan` are
//! literals even where a column could stand, and `INTERVAL` and `CASE` are
//! tried first and read as names if they do not parse.

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::names;
use crate::syntax::grammar::select;
use crate::syntax::grammar::types;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Whether an alias may follow an expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AliasMode {
    /// No alias.
    #[allow(dead_code, reason = "for the statement grammar that builds on it")]
    None,
    /// Only with `AS`, as in function arguments.
    As,
    /// With or without `AS`, as in a `SELECT` list; without it, the alias
    /// cannot be `AS` or one of [`RESTRICTED_ALIASES`].
    Implicit,
    /// As [`Implicit`](Self::Implicit), and without `AS` not one of these
    /// keywords either: the arguments of the functions ClickHouse parses
    /// with syntax of their own, where the words between arguments
    /// (`SUBSTRING(s FROM a FOR b)`) come first.
    ImplicitExcept(&'static [K]),
    /// Only without `AS`: the first argument of `CAST(x AS T)`, where `AS`
    /// starts the type.
    ImplicitOnly,
}

/// The words an alias without `AS` cannot be
/// (`ParserAlias::restricted_keywords`), so that `SELECT x FROM t` does not
/// read `FROM` as an alias.
pub(crate) const RESTRICTED_ALIASES: [&str; 45] = [
    "ALL",
    "ANTI",
    "ANY",
    "ARRAY",
    "ASOF",
    "BETWEEN",
    "CROSS",
    "PASTE",
    "FINAL",
    "FORMAT",
    "FROM",
    "FULL",
    "GLOBAL",
    "GROUP",
    "HAVING",
    "ILIKE",
    "INNER",
    "INTO",
    "JOIN",
    "LEFT",
    "LIKE",
    "LIMIT",
    "NATURAL",
    "NOT",
    "OFFSET",
    "ON",
    "ONLY",
    "ORDER",
    "PARALLEL",
    "PREWHERE",
    "RIGHT",
    "SAMPLE",
    "SEMI",
    "SETTINGS",
    "SIMILAR",
    "STREAM",
    "UNION",
    "USING",
    "WHERE",
    "WINDOW",
    "QUALIFY",
    "WITH",
    "INTERSECT",
    "EXCEPT",
    "ELSE",
];

/// An expression.
pub(crate) fn expr(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    expr_bp(p, 0)
}

/// An expression with an alias, as `mode` allows: an
/// [`AliasedExpr`](SyntaxKind::AliasedExpr) if there is one.
pub(crate) fn expr_with_alias(p: &mut Parser<'_>, mode: AliasMode) -> Option<CompletedMarker> {
    // `SELECT x, y, z, FROM tbl`: a lone `from` takes no alias without `AS`
    // (`ParserWithOptionalAlias`).
    let lone_from = p.at_kw(K::From) && p.nth(1).is_some();
    let start = p.position();
    // Inside brackets, ClickHouse attaches an `AS` alias to the expression
    // so far and goes on: `f(a + b AS x * 2)` is `f((a + b AS x) * 2)`.
    let inline = matches!(mode, AliasMode::As | AliasMode::ImplicitExcept(_));
    let expr = p.nested(|p| expr_bp_inner(p, 0, inline))?;
    let mode = if mode == AliasMode::Implicit
        && lone_from
        && expr.kind() == SyntaxKind::Name
        && p.position() == start + 1
    {
        AliasMode::As
    } else {
        mode
    };
    alias(p, expr, mode)
}

/// The alias of `expr`, if one follows and `mode` allows it.
pub(crate) fn alias(
    p: &mut Parser<'_>,
    expr: CompletedMarker,
    mode: AliasMode,
) -> Option<CompletedMarker> {
    let explicit = !matches!(mode, AliasMode::None | AliasMode::ImplicitOnly)
        && p.at_kw(K::As)
        && names::at_name(p, 1);
    let implicit = match mode {
        AliasMode::None | AliasMode::As => false,
        AliasMode::Implicit | AliasMode::ImplicitOnly => at_implicit_alias(p),
        AliasMode::ImplicitExcept(words) => {
            at_implicit_alias(p) && !p.nth_keyword(0).is_some_and(|k| words.contains(&k))
        }
    };
    if !explicit && !implicit {
        return Some(expr);
    }
    let m = expr.precede(p);
    let a = p.start();
    if explicit {
        p.bump_keyword();
    }
    names::name(p)?;
    a.complete(p, SyntaxKind::Alias);
    Some(m.complete(p, SyntaxKind::AliasedExpr))
}

/// Whether the current token can be an alias without `AS`.
fn at_implicit_alias(p: &Parser<'_>) -> bool {
    match p.nth(0) {
        Some(T::QuotedIdentifier | T::JinjaExpr) => true,
        Some(T::BareWord) => {
            let text = p.nth_text(0);
            if p.at_kw(K::As)
                || RESTRICTED_ALIASES
                    .iter()
                    .any(|w| w.eq_ignore_ascii_case(text))
            {
                return false;
            }
            // `… FROM t COMMENT 'x'` at the end of a statement is a comment
            // clause, not the alias `COMMENT`.
            !(p.at_kw(K::Comment) && p.nth_at(1, T::StringLiteral) && p.nth(2).is_none())
        }
        Some(T::OpeningCurlyBrace) => names::at_query_parameter(p, 0),
        _ => false,
    }
}

/// An expression whose operators bind at least as tightly as `min_bp`.
pub(crate) fn expr_bp(p: &mut Parser<'_>, min_bp: u8) -> Option<CompletedMarker> {
    p.nested(|p| expr_bp_inner(p, min_bp, false))
}

/// [`expr_bp`]; with `inline_alias`, an `AS` alias where no operator
/// follows wraps the expression so far, and operators may follow it.
fn expr_bp_inner(p: &mut Parser<'_>, min_bp: u8, inline_alias: bool) -> Option<CompletedMarker> {
    let mut lhs = prefix(p)?;
    // Each operator applied nests the tree one level deeper, except a run of
    // `AND`s or `OR`s, which is one node.
    let mut chain = 0;
    let mut aliased = false;
    loop {
        let Some(op) = infix(p) else {
            if inline_alias && !aliased && p.at_kw(K::As) && names::at_name(p, 1) {
                lhs = alias(p, lhs, AliasMode::As)?;
                aliased = true;
                continue;
            }
            break;
        };
        if op.bp < min_bp {
            break;
        }
        if op.kind == OpKind::Lambda && !lambda_params(p, lhs) {
            break;
        }
        lhs = p.deeper(chain, |p| apply(p, lhs, op))?;
        if !matches!(op.kind, OpKind::And | OpKind::Or) {
            chain += 1;
        }
    }
    Some(lhs)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpKind {
    /// A binary operator; `quantified` allows `ANY`/`ALL`/`SOME (…)` on the
    /// right, `escape` an `ESCAPE 'c'` after it.
    Binary {
        quantified: bool,
        escape: bool,
    },
    And,
    Or,
    Between,
    Postfix,
    Ternary,
    Lambda,
    Cast,
    Index,
    Dot,
}

#[derive(Clone, Copy, Debug)]
struct Op {
    kind: OpKind,
    /// The left binding power.
    bp: u8,
    /// How many tokens spell the operator.
    tokens: u8,
}

const fn op(kind: OpKind, bp: u8, tokens: u8) -> Op {
    Op { kind, bp, tokens }
}

const fn binary(bp: u8, tokens: u8, quantified: bool, escape: bool) -> Op {
    op(OpKind::Binary { quantified, escape }, bp, tokens)
}

/// The operator at the current token, in operator position.
fn infix(p: &Parser<'_>) -> Option<Op> {
    Some(match p.nth(0)? {
        T::Arrow => op(OpKind::Lambda, 2, 1),
        T::QuestionMark => op(OpKind::Ternary, 4, 1),
        T::PipeMark => binary(16, 1, false, false),
        T::Equals
        | T::NotEquals
        | T::Less
        | T::Greater
        | T::LessOrEquals
        | T::GreaterOrEquals
        | T::Spaceship
        | T::Tilde
        | T::TildeAsterisk
        | T::NotTilde
        | T::NotTildeAsterisk => binary(18, 1, true, false),
        T::Concatenation => binary(20, 1, false, false),
        T::Plus | T::Minus => binary(22, 1, false, false),
        T::Asterisk | T::Slash | T::Percent => binary(24, 1, false, false),
        T::Dot => op(OpKind::Dot, 28, 1),
        T::OpeningSquareBracket => op(OpKind::Index, 28, 1),
        T::DoubleColon => op(OpKind::Cast, 28, 1),
        T::BareWord => return keyword_infix(p),
        _ => return None,
    })
}

fn keyword_infix(p: &Parser<'_>) -> Option<Op> {
    Some(match p.nth_keyword(0)? {
        K::Or => op(OpKind::Or, 6, 1),
        K::And => op(OpKind::And, 8, 1),
        K::Is => {
            let not = u8::from(p.nth_at_kw(1, K::Not));
            let next = usize::from(1 + not);
            match p.nth_keyword(next) {
                Some(K::Null | K::True | K::False | K::Unknown) => op(OpKind::Postfix, 12, 2 + not),
                Some(K::Distinct) if p.nth_at_kw(next + 1, K::From) => {
                    binary(12, 3 + not, true, false)
                }
                _ => return None,
            }
        }
        K::Between => op(OpKind::Between, 14, 1),
        K::Not => match p.nth_keyword(1)? {
            K::Between => op(OpKind::Between, 14, 2),
            K::Like | K::Ilike => binary(18, 2, true, true),
            K::In => binary(18, 2, false, false),
            K::Similar if p.nth_at_kw(2, K::To) => binary(18, 3, true, true),
            _ => return None,
        },
        K::Like | K::Ilike => binary(18, 1, true, true),
        K::Regexp => binary(18, 1, true, false),
        K::Similar if p.nth_at_kw(1, K::To) => binary(18, 2, true, true),
        // `IN PARTITION` belongs to `ALTER … IN PARTITION p`.
        K::In if !p.no_in() && !p.nth_at_kw(1, K::Partition) => binary(18, 1, false, false),
        K::Global => match (p.nth_keyword(1), p.nth_keyword(2)) {
            (Some(K::In), _) => binary(18, 2, false, false),
            (Some(K::Not), Some(K::In)) => binary(18, 3, false, false),
            _ => return None,
        },
        K::Mod | K::Div => binary(24, 1, false, false),
        K::At => match p.nth_keyword(1) {
            Some(K::Time) if p.nth_at_kw(2, K::Zone) => binary(26, 3, false, false),
            Some(K::Local) => op(OpKind::Postfix, 26, 2),
            _ => return None,
        },
        _ => return None,
    })
}

fn bump_operator(p: &mut Parser<'_>, tokens: u8) {
    for _ in 0..tokens {
        p.bump_keyword();
    }
}

fn apply(p: &mut Parser<'_>, lhs: CompletedMarker, op: Op) -> Option<CompletedMarker> {
    match op.kind {
        OpKind::Binary { quantified, escape } => {
            let m = lhs.precede(p);
            bump_operator(p, op.tokens);
            if quantified && at_quantifier(p) {
                quantified_expr(p)?;
            } else {
                expr_bp(p, op.bp + 1)?;
            }
            if escape && p.at_kw(K::Escape) && p.nth_at(1, T::StringLiteral) {
                p.bump_keyword();
                literal(p);
            }
            Some(m.complete(p, SyntaxKind::BinaryExpr))
        }
        OpKind::And | OpKind::Or => {
            let (keyword, kind) = if op.kind == OpKind::And {
                (K::And, SyntaxKind::AndExpr)
            } else {
                (K::Or, SyntaxKind::OrExpr)
            };
            let m = lhs.precede(p);
            while p.at_kw(keyword) {
                p.bump_keyword();
                expr_bp(p, op.bp + 1)?;
            }
            Some(m.complete(p, kind))
        }
        OpKind::Between => {
            let m = lhs.precede(p);
            bump_operator(p, op.tokens);
            expr_bp(p, 16)?;
            p.expect_kw(K::And)?;
            expr_bp(p, 18)?;
            Some(m.complete(p, SyntaxKind::BetweenExpr))
        }
        OpKind::Postfix => {
            let m = lhs.precede(p);
            bump_operator(p, op.tokens);
            Some(m.complete(p, SyntaxKind::PostfixExpr))
        }
        OpKind::Ternary => {
            let m = lhs.precede(p);
            p.bump();
            expr_bp(p, 6)?;
            p.expect(T::Colon)?;
            expr_bp(p, 8)?;
            Some(m.complete(p, SyntaxKind::TernaryExpr))
        }
        OpKind::Lambda => {
            let m = lhs.precede(p);
            p.bump();
            expr_bp(p, 3)?;
            Some(m.complete(p, SyntaxKind::LambdaExpr))
        }
        OpKind::Cast => {
            let operand = literal_as_text(p, lhs);
            let m = operand.precede(p);
            p.bump();
            types::data_type(p)?;
            Some(m.complete(p, SyntaxKind::CastExpr))
        }
        OpKind::Index => {
            let m = lhs.precede(p);
            p.bump();
            // `json.a[]` reads an `Array(JSON)` subcolumn.
            if !p.at(T::ClosingSquareBracket) {
                p.with_no_in(false, expr)?;
            }
            p.expect(T::ClosingSquareBracket)?;
            Some(m.complete(p, SyntaxKind::IndexExpr))
        }
        OpKind::Dot => dot(p, lhs),
    }
}

/// `.` after an operand: `x.1`, `f(x).name`, `t.*`, `t.COLUMNS(…)`, and
/// any other operand ClickHouse takes as an element, `x.-1` or `x.(e)`.
fn dot(p: &mut Parser<'_>, lhs: CompletedMarker) -> Option<CompletedMarker> {
    match p.nth(1) {
        Some(T::Asterisk) => asterisk(p, Some(lhs)),
        _ if p.nth_text(1) == "COLUMNS" && at_columns_matcher(p, 1) => {
            columns_matcher(p, Some(lhs))
        }
        Some(T::Number) => {
            let m = lhs.precede(p);
            p.bump();
            p.bump();
            Some(m.complete(p, SyntaxKind::TupleAccessExpr))
        }
        _ if names::at_name(p, 1) => {
            let m = lhs.precede(p);
            p.bump();
            names::name(p)?;
            Some(m.complete(p, SyntaxKind::TupleAccessExpr))
        }
        _ => {
            let m = lhs.precede(p);
            p.bump();
            expr_bp(p, 29)?;
            Some(m.complete(p, SyntaxKind::TupleAccessExpr))
        }
    }
}

/// Whether `lhs` can be a lambda's parameters: a name, or names in
/// parentheses.
fn lambda_params(p: &Parser<'_>, lhs: CompletedMarker) -> bool {
    match lhs.kind() {
        SyntaxKind::Name => true,
        SyntaxKind::ParenExpr | SyntaxKind::TupleExpr => p
            .children(lhs)
            .iter()
            .all(|kind| kind.is_token() || *kind == SyntaxKind::Name),
        _ => false,
    }
}

/// `operand` wrapped in a [`LiteralAsText`](SyntaxKind::LiteralAsText) if it
/// is a literal ClickHouse reads as text: a number, string or `NULL`, a
/// negative number, or an array or tuple of those. Its `NULL`s are not
/// keyword slots then, since its bytes are kept.
fn literal_as_text(p: &mut Parser<'_>, operand: CompletedMarker) -> CompletedMarker {
    let shape = |kind: SyntaxKind| {
        matches!(
            kind,
            SyntaxKind::Literal
                | SyntaxKind::PrefixExpr
                | SyntaxKind::ArrayExpr
                | SyntaxKind::TupleExpr
                | SyntaxKind::ParenExpr
        )
    };
    let literal = shape(operand.kind())
        && p.descendants(operand).all(|kind| {
            shape(kind)
                || matches!(
                    kind,
                    SyntaxKind::Number
                        | SyntaxKind::StringLiteral
                        | SyntaxKind::Minus
                        | SyntaxKind::Comma
                        | SyntaxKind::Keyword
                        | SyntaxKind::OpeningRoundBracket
                        | SyntaxKind::ClosingRoundBracket
                        | SyntaxKind::OpeningSquareBracket
                        | SyntaxKind::ClosingSquareBracket
                )
        });
    if literal {
        p.demote_keywords(operand);
        operand.precede(p).complete(p, SyntaxKind::LiteralAsText)
    } else {
        operand
    }
}

fn at_quantifier(p: &Parser<'_>) -> bool {
    match p.nth_keyword(0) {
        // `x = any(y)` is the aggregate function unless `y` is a subquery.
        Some(K::Any) => select::at_subquery(p, 1),
        Some(K::Some | K::All) => p.nth_at(1, T::OpeningRoundBracket),
        _ => false,
    }
}

fn quantified_expr(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.bump_keyword();
    paren(p)?;
    Some(m.complete(p, SyntaxKind::QuantifiedExpr))
}

// ---- Operands -------------------------------------------------------------

/// A prefix operator and its operand, or an atom.
fn prefix(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    match p.nth(0) {
        Some(T::Minus) if p.nth_at(1, T::Number) || at_number_word(p, 1) => {
            // A negative number is one literal to ClickHouse, so postfix
            // operators apply to it whole: `-1::Int8` casts `-1`.
            let m = p.start();
            p.bump();
            literal(p);
            Some(m.complete(p, SyntaxKind::PrefixExpr))
        }
        Some(T::Minus | T::Plus) => prefix_op(p, 27),
        _ if p.at_kw(K::Not) => prefix_op(p, 11),
        _ => atom(p),
    }
}

fn prefix_op(p: &mut Parser<'_>, bp: u8) -> Option<CompletedMarker> {
    let m = p.start();
    p.bump_keyword();
    expr_bp(p, bp)?;
    Some(m.complete(p, SyntaxKind::PrefixExpr))
}

/// An operand that is not a prefix operator.
fn atom(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    match p.nth(0) {
        Some(T::Number | T::StringLiteral | T::HereDoc) => literal(p),
        Some(T::OpeningRoundBracket) => paren(p),
        Some(T::OpeningSquareBracket) => array(p, false),
        Some(T::OpeningCurlyBrace) if names::at_query_parameter(p, 0) => name_or_call(p),
        Some(T::OpeningCurlyBrace) => map(p),
        Some(T::Asterisk) => asterisk(p, None),
        Some(T::DoubleAt) => {
            let m = p.start();
            p.bump();
            names::name(p)?;
            Some(m.complete(p, SyntaxKind::GlobalVariable))
        }
        Some(T::BareWord) => word(p),
        Some(T::QuotedIdentifier | T::JinjaExpr | T::Verbatim) => name_or_call(p),
        _ => p.expected("an expression"),
    }
}

/// An operand that starts with a bare word.
fn word(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    match p.nth_keyword(0) {
        Some(K::Null | K::True | K::False) => return literal(p),
        Some(K::Interval) => {
            if let Some(interval) = p.speculate(interval) {
                return Some(interval);
            }
        }
        Some(K::Case) => {
            if let Some(case) = p.speculate(case) {
                return Some(case);
            }
        }
        Some(K::Date | K::Timestamp) if p.nth_at(1, T::StringLiteral) => {
            let m = p.start();
            p.bump_keyword();
            literal(p);
            return Some(m.complete(p, SyntaxKind::TypedLiteral));
        }
        Some(K::Columns) if at_columns_matcher(p, 0) => {
            return columns_matcher(p, None);
        }
        Some(K::Array) if p.nth_at(1, T::OpeningSquareBracket) && array_constructor(p) => {
            return array(p, true);
        }
        _ => {}
    }
    if at_number_word(p, 0) {
        return literal(p);
    }
    name_or_call(p)
}

/// Whether the `n`th token is `inf`, `infinity` or `nan`, which ClickHouse
/// reads as numbers before it considers names.
fn at_number_word(p: &Parser<'_>, n: usize) -> bool {
    p.nth_at(n, T::BareWord)
        && ["inf", "infinity", "nan"]
            .iter()
            .any(|w| w.eq_ignore_ascii_case(p.nth_text(n)))
}

/// A [`Literal`](SyntaxKind::Literal) of the current token: a number, string
/// or heredoc, `NULL`, or a literal word.
pub(crate) fn literal(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    match p.nth(0) {
        Some(T::Number | T::StringLiteral | T::HereDoc) => p.bump(),
        Some(T::BareWord) if p.at_kw(K::Null) => p.bump_keyword(),
        Some(T::BareWord) if p.at_kw(K::True) || p.at_kw(K::False) || at_number_word(p, 0) => {
            p.bump_as(SyntaxKind::LiteralWord);
        }
        _ => {
            m.abandon(p);
            return p.expected("a literal");
        }
    }
    Some(m.complete(p, SyntaxKind::Literal))
}

/// Whether the `n`th token starts a literal.
pub(crate) fn at_literal(p: &Parser<'_>, n: usize) -> bool {
    match p.nth(n) {
        Some(T::Number | T::StringLiteral | T::HereDoc) => true,
        Some(T::Minus | T::Plus) => p.nth_at(n + 1, T::Number) || at_number_word(p, n + 1),
        Some(T::BareWord) => {
            matches!(p.nth_keyword(n), Some(K::Null | K::True | K::False)) || at_number_word(p, n)
        }
        _ => false,
    }
}

/// A name, a dotted name, or a function call.
fn name_or_call(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let call = p.nth_at(1, T::OpeningRoundBracket).then(|| p.nth_text(0));
    let name = names::compound_name_with(p, true)?;
    if p.at(T::OpeningRoundBracket) {
        let single = call.filter(|_| name.kind() == SyntaxKind::Name);
        return function_call(p, name, single);
    }
    Some(name)
}

/// Functions that ClickHouse parses with syntax of their own
/// (`getFunctionLayer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Special {
    None,
    Cast,
    Extract,
    Substring,
    Overlay,
    Position,
    Trim,
    TrimSide,
    DateUnit,
    Exists,
}

fn special(name: &str) -> Special {
    let is = |names: &[&str]| names.iter().any(|n| n.eq_ignore_ascii_case(name));
    if is(&["cast", "try_cast"]) {
        Special::Cast
    } else if is(&["extract"]) {
        Special::Extract
    } else if is(&["substring"]) {
        Special::Substring
    } else if is(&["overlay", "overlayutf8"]) {
        Special::Overlay
    } else if is(&["position"]) {
        Special::Position
    } else if is(&["trim"]) {
        Special::Trim
    } else if is(&["ltrim", "rtrim"]) {
        Special::TrimSide
    } else if is(&[
        "dateadd",
        "date_add",
        "timestampadd",
        "timestamp_add",
        "datesub",
        "date_sub",
        "timestampsub",
        "timestamp_sub",
        "datediff",
        "date_diff",
        "timestampdiff",
        "timestamp_diff",
    ]) {
        Special::DateUnit
    } else if is(&["exists"]) {
        Special::Exists
    } else {
        Special::None
    }
}

/// A function call whose name, `name`, has been parsed; `single` is the
/// name's text when it is one word.
fn function_call(
    p: &mut Parser<'_>,
    name: CompletedMarker,
    single: Option<&str>,
) -> Option<CompletedMarker> {
    let m = name.precede(p);
    match single.map_or(Special::None, special) {
        Special::Cast => cast_args(p)?,
        Special::Extract => extract_args(p)?,
        Special::Substring => substring_args(p)?,
        Special::Overlay => overlay_args(p)?,
        Special::Position => position_args(p)?,
        Special::Trim => trim_args(p, true)?,
        Special::TrimSide => trim_args(p, false)?,
        Special::DateUnit => date_unit_args(p)?,
        Special::Exists if select::at_subquery(p, 0) => {
            select::subquery(p)?;
        }
        Special::Exists | Special::None => {
            arg_list(p)?;
            if p.at(T::OpeningRoundBracket) {
                arg_list(p)?;
            }
        }
    }
    if p.at_kw(K::Filter) && p.nth_at(1, T::OpeningRoundBracket) && p.nth_at_kw(2, K::Where) {
        let f = p.start();
        p.bump_keyword();
        p.bump();
        p.bump_keyword();
        expr(p)?;
        p.expect(T::ClosingRoundBracket)?;
        f.complete(p, SyntaxKind::FunctionFilter);
    }
    if (p.at_kw(K::Respect) || p.at_kw(K::Ignore)) && p.nth_at_kw(1, K::Nulls) {
        let n = p.start();
        p.bump_keyword();
        p.bump_keyword();
        n.complete(p, SyntaxKind::NullsModifier);
    }
    if p.at_kw(K::Over) && (p.nth_at(1, T::OpeningRoundBracket) || names::at_name(p, 1)) {
        let o = p.start();
        p.bump_keyword();
        if p.at(T::OpeningRoundBracket) {
            fallback::opaque_group(p)?;
        } else {
            names::name(p)?;
        }
        o.complete(p, SyntaxKind::Over);
    }
    Some(m.complete(p, SyntaxKind::FunctionCall))
}

/// `( [DISTINCT | ALL] a, b AS c, … )`.
pub(crate) fn arg_list(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    call_args(p, |p| {
        // `f(DISTINCT)` and `f(ALL, x)` name columns.
        if (p.at_kw(K::Distinct) || p.at_kw(K::All))
            && !matches!(p.nth(1), Some(T::Comma | T::ClosingRoundBracket))
        {
            p.bump_keyword();
        }
        arguments(p, AliasMode::As);
        Some(SyntaxKind::ArgList)
    })
}

/// `( … )` after a function's name, as the node kind `body` returns; `body`
/// parses what lies between the brackets, where `IN` is an operator again.
fn call_args(
    p: &mut Parser<'_>,
    body: impl FnOnce(&mut Parser<'_>) -> Option<SyntaxKind>,
) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect(T::OpeningRoundBracket)?;
    let kind = p.with_no_in(false, body)?;
    p.expect(T::ClosingRoundBracket)?;
    Some(m.complete(p, kind))
}

/// Arguments up to `)`, each with an alias as `alias` allows.
fn arguments(p: &mut Parser<'_>, alias: AliasMode) {
    fallback::comma_separated(p, &fallback::never, &mut |p| {
        expr_with_alias(p, alias).map(drop)
    });
}

/// More arguments, if a comma follows those parsed, of a function with
/// syntax of its own that is written with commas: an
/// [`ArgList`](SyntaxKind::ArgList).
fn more_args(p: &mut Parser<'_>) -> SyntaxKind {
    if p.eat(T::Comma) {
        arguments(p, AliasMode::Implicit);
    }
    SyntaxKind::ArgList
}

/// An argument of a function with syntax of its own, which may have an
/// alias, with `AS` or without, as long as it is not one of `separators`.
fn special_arg(p: &mut Parser<'_>, separators: &'static [K]) -> Option<CompletedMarker> {
    expr_with_alias(p, AliasMode::ImplicitExcept(separators))
}

/// The comma-separated arguments of a function with syntax of its own,
/// which take aliases without `AS`.
fn special_arg_list(p: &mut Parser<'_>) -> Option<()> {
    call_args(p, |p| {
        arguments(p, AliasMode::Implicit);
        Some(SyntaxKind::ArgList)
    })
    .map(drop)
}

/// `CAST(x AS T)`, `CAST(x AS alias AS T)`, `CAST(x, 'T')`.
fn cast_args(p: &mut Parser<'_>) -> Option<()> {
    call_args(p, |p| {
        let mut operand = expr(p)?;
        if p.at_kw(K::As) || p.at(T::Comma) {
            operand = literal_as_text(p, operand);
        }
        operand = alias(p, operand, AliasMode::ImplicitOnly)?;
        if p.at_kw(K::As)
            && names::at_name(p, 1)
            && (p.nth_at_kw(2, K::As) || p.nth_at(2, T::Comma))
        {
            alias(p, operand, AliasMode::As)?;
        }
        if p.eat_kw(K::As) {
            types::data_type(p)?;
            return Some(SyntaxKind::KeywordArgList);
        }
        Some(more_args(p))
    })
    .map(drop)
}

/// `EXTRACT(DAY FROM d)`, or `extract(haystack, pattern)`.
fn extract_args(p: &mut Parser<'_>) -> Option<()> {
    if !((at_interval_unit(p, 1) || at_extract_unit(p, 1)) && p.nth_at_kw(2, K::From)) {
        return special_arg_list(p);
    }
    call_args(p, |p| {
        interval_unit(p);
        p.bump_keyword();
        special_arg(p, &[])?;
        Some(SyntaxKind::KeywordArgList)
    })
    .map(drop)
}

/// `SUBSTRING(s FROM a [FOR b])`, `substring(s, a[, b])`, and the mixtures
/// ClickHouse accepts.
fn substring_args(p: &mut Parser<'_>) -> Option<()> {
    call_args(p, |p| {
        let mut keywords = false;
        if !p.at(T::ClosingRoundBracket) {
            special_arg(p, &[K::For])?;
            for separator in [K::From, K::For] {
                if p.at_kw(separator) {
                    p.bump_keyword();
                    keywords = true;
                } else if !p.eat(T::Comma) {
                    break;
                }
                special_arg(p, &[K::For])?;
            }
        }
        Some(if keywords {
            SyntaxKind::KeywordArgList
        } else {
            more_args(p)
        })
    })
    .map(drop)
}

/// `OVERLAY(s PLACING r FROM a [FOR b])` or `overlay(s, r, a[, b])`.
fn overlay_args(p: &mut Parser<'_>) -> Option<()> {
    if !overlay_keyword_form(p) {
        return arg_list(p).map(drop);
    }
    call_args(p, |p| {
        let separators = &[K::Placing, K::For];
        special_arg(p, separators)?;
        p.expect_kw(K::Placing)?;
        special_arg(p, separators)?;
        p.expect_kw(K::From)?;
        special_arg(p, separators)?;
        if p.eat_kw(K::For) {
            special_arg(p, separators)?;
        }
        Some(SyntaxKind::KeywordArgList)
    })
    .map(drop)
}

/// Whether `PLACING` follows the first argument outside brackets, before
/// the first comma.
fn overlay_keyword_form(p: &Parser<'_>) -> bool {
    let mut depth = 0usize;
    let mut n = 1;
    while let Some(kind) = p.nth(n) {
        if fallback::is_opener(kind) {
            depth += 1;
        } else if fallback::is_closer(kind) {
            if depth == 0 {
                return false;
            }
            depth -= 1;
        } else if depth == 0 && (kind == T::Comma || n > 10_000) {
            return false;
        } else if depth == 0 && p.nth_at_kw(n, K::Placing) {
            return true;
        }
        n += 1;
    }
    false
}

/// `POSITION(needle IN haystack)` or `position(haystack, needle)`.
fn position_args(p: &mut Parser<'_>) -> Option<()> {
    call_args(p, |p| {
        p.with_no_in(true, |p| special_arg(p, &[K::In]))?;
        if p.at_kw(K::In) {
            p.bump_keyword();
            special_arg(p, &[])?;
            return Some(SyntaxKind::KeywordArgList);
        }
        Some(more_args(p))
    })
    .map(drop)
}

/// `TRIM(BOTH|LEADING|TRAILING [chars] FROM s)`, or `trim(s)`; `full` is
/// false for `ltrim` and `rtrim`, which take no keywords.
fn trim_args(p: &mut Parser<'_>, full: bool) -> Option<()> {
    if !(full && matches!(p.nth_keyword(1), Some(K::Both | K::Leading | K::Trailing))) {
        return special_arg_list(p);
    }
    call_args(p, |p| {
        p.bump_keyword();
        if !p.at_kw(K::From) {
            special_arg(p, &[])?;
        }
        p.expect_kw(K::From)?;
        special_arg(p, &[])?;
        Some(SyntaxKind::KeywordArgList)
    })
    .map(drop)
}

/// `dateAdd(DAY, n, d)` and its relatives, whose first argument may be an
/// interval unit.
fn date_unit_args(p: &mut Parser<'_>) -> Option<()> {
    if !(at_interval_unit(p, 1) && p.nth_at(2, T::Comma)) {
        return special_arg_list(p);
    }
    call_args(p, |p| {
        interval_unit(p);
        Some(more_args(p))
    })
    .map(drop)
}

/// Whether the `n`th token is an interval unit (`parseIntervalKind`).
pub(crate) fn at_interval_unit(p: &Parser<'_>, n: usize) -> bool {
    matches!(
        p.nth_keyword(n),
        Some(
            K::Nanosecond
                | K::Nanoseconds
                | K::SqlTsiNanosecond
                | K::Ns
                | K::Microsecond
                | K::Microseconds
                | K::SqlTsiMicrosecond
                | K::Millisecond
                | K::Milliseconds
                | K::SqlTsiMillisecond
                | K::Ms
                | K::Second
                | K::Seconds
                | K::SqlTsiSecond
                | K::Ss
                | K::S
                | K::Minute
                | K::Minutes
                | K::SqlTsiMinute
                | K::Mi
                | K::N
                | K::Hour
                | K::Hours
                | K::SqlTsiHour
                | K::Hh
                | K::H
                | K::Day
                | K::Days
                | K::SqlTsiDay
                | K::Dd
                | K::D
                | K::Week
                | K::Weeks
                | K::SqlTsiWeek
                | K::Wk
                | K::Ww
                | K::Month
                | K::Months
                | K::SqlTsiMonth
                | K::Mm
                | K::M
                | K::Quarter
                | K::Quarters
                | K::SqlTsiQuarter
                | K::Qq
                | K::Q
                | K::Year
                | K::Years
                | K::SqlTsiYear
                | K::Yyyy
                | K::Yy
        )
    )
}

/// Whether the `n`th token is a unit only `EXTRACT` takes.
fn at_extract_unit(p: &Parser<'_>, n: usize) -> bool {
    matches!(
        p.nth_keyword(n),
        Some(
            K::Epoch
                | K::Dow
                | K::Doy
                | K::Isodow
                | K::Isoyear
                | K::Century
                | K::Decade
                | K::Millennium
                | K::TimezoneHour
                | K::TimezoneMinute
        )
    )
}

/// An [`IntervalUnit`](SyntaxKind::IntervalUnit) of the current token.
fn interval_unit(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump_keyword();
    m.complete(p, SyntaxKind::IntervalUnit);
}

/// `INTERVAL n UNIT`, `INTERVAL 'n UNIT …'`, `INTERVAL 'n' UNIT [TO UNIT]`.
fn interval(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.bump_keyword();
    if p.at(T::StringLiteral) {
        let value = literal(p)?;
        alias(p, value, AliasMode::As)?;
        if at_interval_unit(p, 0) {
            interval_unit(p);
            if p.at_kw(K::To) && at_interval_unit(p, 1) {
                p.bump_keyword();
                interval_unit(p);
            }
        }
    } else {
        let value = expr(p)?;
        alias(p, value, AliasMode::As)?;
        if !at_interval_unit(p, 0) {
            return p.expected("an interval unit");
        }
        interval_unit(p);
    }
    Some(m.complete(p, SyntaxKind::IntervalExpr))
}

/// `CASE [x] WHEN a THEN b … [ELSE c] END`.
fn case(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.bump_keyword();
    if !p.at_kw(K::When) {
        expr(p)?;
    }
    while p.at_kw(K::When) {
        let w = p.start();
        p.bump_keyword();
        expr(p)?;
        p.expect_kw(K::Then)?;
        expr(p)?;
        w.complete(p, SyntaxKind::CaseWhen);
    }
    if p.at_kw(K::Else) {
        let e = p.start();
        p.bump_keyword();
        expr(p)?;
        e.complete(p, SyntaxKind::CaseElse);
    }
    p.expect_kw(K::End)?;
    Some(m.complete(p, SyntaxKind::CaseExpr))
}

/// `( … )`: a subquery, a parenthesised expression, or a tuple.
fn paren(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if select::at_subquery(p, 0) {
        return select::subquery(p);
    }
    let m = p.start();
    p.expect(T::OpeningRoundBracket)?;
    let shape = p.with_no_in(false, |p| {
        Some(fallback::comma_separated(p, &fallback::never, &mut |p| {
            expr_with_alias(p, AliasMode::As).map(drop)
        }))
    })?;
    p.expect(T::ClosingRoundBracket)?;
    let kind = if shape.items == 1 && shape.commas == 0 {
        SyntaxKind::ParenExpr
    } else {
        SyntaxKind::TupleExpr
    };
    Some(m.complete(p, kind))
}

/// `[ a, b ]`, or `ARRAY[ a, b ]` when `keyword` is set.
fn array(p: &mut Parser<'_>, keyword: bool) -> Option<CompletedMarker> {
    let m = p.start();
    if keyword {
        p.bump_keyword();
    }
    p.expect(T::OpeningSquareBracket)?;
    p.with_no_in(false, |p| {
        fallback::comma_separated(p, &fallback::never, &mut |p| {
            expr_with_alias(p, AliasMode::As).map(drop)
        });
        Some(())
    })?;
    p.expect(T::ClosingSquareBracket)?;
    Some(m.complete(p, SyntaxKind::ArrayExpr))
}

/// Whether `ARRAY[…]` at the current token constructs an array: it is empty,
/// or has a comma outside brackets. `ARRAY[x]` with one element subscripts a
/// column named `array`, as ClickHouse reads it.
fn array_constructor(p: &Parser<'_>) -> bool {
    if p.nth_at(2, T::ClosingSquareBracket) {
        return true;
    }
    let mut depth = 0usize;
    // The bound keeps nested `ARRAY[` from scanning quadratically; an array
    // that long before its first comma is read as a subscript.
    for n in 2..10_000 {
        let Some(kind) = p.nth(n) else {
            return false;
        };
        if fallback::is_opener(kind) {
            depth += 1;
        } else if fallback::is_closer(kind) {
            if depth == 0 {
                return false;
            }
            depth -= 1;
        } else if kind == T::Comma && depth == 0 {
            return true;
        }
    }
    false
}

/// `{ key: value, … }`.
fn map(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect(T::OpeningCurlyBrace)?;
    p.with_no_in(false, |p| {
        fallback::comma_separated(p, &fallback::never, &mut |p| {
            let e = p.start();
            expr(p)?;
            p.expect(T::Colon)?;
            expr(p)?;
            e.complete(p, SyntaxKind::MapEntry);
            Some(())
        });
        Some(())
    })?;
    p.expect(T::ClosingCurlyBrace)?;
    Some(m.complete(p, SyntaxKind::MapExpr))
}

/// `{name:Type}`.
pub(crate) fn query_parameter(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect(T::OpeningCurlyBrace)?;
    names::name(p)?;
    p.expect(T::Colon)?;
    types::data_type(p)?;
    p.expect(T::ClosingCurlyBrace)?;
    Some(m.complete(p, SyntaxKind::QueryParameter))
}

/// `*` or `qualifier.*`, then `LIKE '…'` and column transformers.
fn asterisk(p: &mut Parser<'_>, qualifier: Option<CompletedMarker>) -> Option<CompletedMarker> {
    let m = match qualifier {
        Some(qualifier) => {
            let m = qualifier.precede(p);
            p.bump();
            m
        }
        None => p.start(),
    };
    p.expect(T::Asterisk)?;
    if (p.at_kw(K::Like) || p.at_kw(K::Ilike)) && p.nth_at(1, T::StringLiteral) {
        p.bump_keyword();
        literal(p);
    }
    column_transformers(p)?;
    Some(m.complete(p, SyntaxKind::AsteriskExpr))
}

/// Whether the `n`th token, `COLUMNS`, starts a columns matcher: `(`, a
/// string or dotted names separated by commas, then `)`. Anything else in
/// the brackets makes it a call of a function named `columns`, as
/// ClickHouse reads it.
pub(crate) fn at_columns_matcher(p: &Parser<'_>, n: usize) -> bool {
    if !p.nth_at(n + 1, T::OpeningRoundBracket) {
        return false;
    }
    if p.nth_at(n + 2, T::StringLiteral) {
        return p.nth_at(n + 3, T::ClosingRoundBracket);
    }
    let mut i = n + 2;
    loop {
        let Some(name) = names::name_tokens(p, i) else {
            return false;
        };
        i += name;
        match p.nth(i) {
            Some(T::Dot | T::Comma) => i += 1,
            Some(T::ClosingRoundBracket) => return true,
            _ => return false,
        }
    }
}

/// `COLUMNS(…)` or `qualifier.COLUMNS(…)`, then column transformers; see
/// [`at_columns_matcher`].
fn columns_matcher(
    p: &mut Parser<'_>,
    qualifier: Option<CompletedMarker>,
) -> Option<CompletedMarker> {
    let m = match qualifier {
        Some(qualifier) => {
            let m = qualifier.precede(p);
            p.bump();
            m
        }
        None => p.start(),
    };
    p.bump_keyword();
    let list = p.start();
    p.expect(T::OpeningRoundBracket)?;
    if p.at(T::StringLiteral) {
        literal(p)?;
    } else {
        names::compound_name(p)?;
        while p.eat(T::Comma) {
            names::compound_name(p)?;
        }
    }
    p.expect(T::ClosingRoundBracket)?;
    list.complete(p, SyntaxKind::ColumnList);
    column_transformers(p)?;
    Some(m.complete(p, SyntaxKind::ColumnsMatcher))
}

fn column_transformers(p: &mut Parser<'_>) -> Option<()> {
    loop {
        let keyword = p.nth_keyword(0);
        if !matches!(keyword, Some(K::Apply | K::Except | K::Replace)) {
            return Some(());
        }
        let m = p.start();
        p.bump_keyword();
        match keyword {
            Some(K::Apply) => {
                expr(p)?;
            }
            Some(K::Except) => {
                p.eat_kw(K::Strict);
                prefix(p)?;
            }
            _ => {
                p.eat_kw(K::Strict);
                if p.at(T::OpeningRoundBracket) {
                    paren(p)?;
                } else {
                    expr_with_alias(p, AliasMode::As)?;
                }
            }
        }
        m.complete(p, SyntaxKind::ColumnTransformer);
    }
}

#[cfg(test)]
mod tests;
