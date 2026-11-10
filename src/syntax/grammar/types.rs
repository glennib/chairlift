//! Data types, as `ParserDataType` reads them: a name of one or more words,
//! then arguments in parentheses whose grammar depends on the name.
//!
//! Type names keep their case, so every word of one is a
//! [`BareWord`](crate::syntax::SyntaxKind::BareWord); keywords inside the
//! arguments (`SKIP` in `JSON(…)`) are keyword slots.

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::names;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Words that are never a type's name, because a column definition reads
/// them as the keyword that may follow the name when the type is left out
/// (`x DEFAULT 1`, `x NULL`).
const NOT_TYPE_NAMES: [&str; 13] = [
    "NOT",
    "NULL",
    "DEFAULT",
    "MATERIALIZED",
    "EPHEMERAL",
    "ALIAS",
    "AUTO",
    "PRIMARY",
    "TTL",
    "COMMENT",
    "CODEC",
    "SETTINGS",
    "STATISTICS",
];

/// Integer type names that take a display width `(N)` and `SIGNED` or
/// `UNSIGNED`.
const INTEGER_NAMES: [&str; 19] = [
    "INT8",
    "INT16",
    "INT32",
    "INT64",
    "INT128",
    "INT256",
    "UINT8",
    "UINT16",
    "UINT32",
    "UINT64",
    "UINT128",
    "UINT256",
    "TINYINT",
    "SMALLINT",
    "MEDIUMINT",
    "INT",
    "INTEGER",
    "BIGINT",
    "INT1",
];

fn any_ci(words: &[&str], text: &str) -> bool {
    words.iter().any(|w| w.eq_ignore_ascii_case(text))
}

/// Whether the `n`th token can start a data type.
pub(crate) fn at_data_type(p: &Parser<'_>, n: usize) -> bool {
    match p.nth(n) {
        Some(T::BareWord) => !any_ci(&NOT_TYPE_NAMES, p.nth_text(n)),
        Some(T::QuotedIdentifier | T::JinjaExpr) => true,
        _ => false,
    }
}

/// A [`DataType`](SyntaxKind::DataType).
pub(crate) fn data_type(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    p.nested(data_type_inner)
}

fn data_type_inner(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !at_data_type(p, 0) {
        return p.expected("a data type");
    }
    let name = p.nth_text(0);
    let m = p.start();
    let n = p.start();
    p.bump();
    let integer = any_ci(&INTEGER_NAMES, name);
    if name.eq_ignore_ascii_case("NATIONAL") {
        let _ = eat_words(p, &[K::Character, K::Large, K::Object])
            || eat_words(p, &[K::Character, K::Varying])
            || eat_words(p, &[K::Char, K::Varying])
            || eat_words(p, &[K::Character])
            || eat_words(p, &[K::Char]);
    } else if any_ci(&["BINARY", "CHARACTER", "CHAR", "NCHAR"], name) {
        let _ = eat_words(p, &[K::Large, K::Object]) || eat_words(p, &[K::Varying]);
    } else if name.eq_ignore_ascii_case("DOUBLE") {
        eat_words(p, &[K::Precision]);
    } else if integer {
        let _ = eat_words(p, &[K::Signed]) || eat_words(p, &[K::Unsigned]);
    }
    n.complete(p, SyntaxKind::Name);
    if p.at(T::OpeningRoundBracket) {
        type_args(p, name)?;
        if integer {
            let _ = eat_words(p, &[K::Signed]) || eat_words(p, &[K::Unsigned]);
        }
    }
    Some(m.complete(p, SyntaxKind::DataType))
}

/// Consumes `words` as part of a type's name, if they follow.
fn eat_words(p: &mut Parser<'_>, words: &[K]) -> bool {
    let at = p.at_kws(words);
    if at {
        for _ in words {
            p.bump();
        }
    }
    at
}

/// `( … )` after a type's name.
fn type_args(p: &mut Parser<'_>, name: &str) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect(T::OpeningRoundBracket)?;
    let mut index = 0usize;
    let mut function_at = 0usize;
    fallback::comma_separated(p, &fallback::never, &mut |p| {
        let result = type_arg(p, name, index, &mut function_at);
        index += 1;
        result
    });
    p.expect(T::ClosingRoundBracket)?;
    Some(m.complete(p, SyntaxKind::TypeArgList))
}

/// One argument of the type `name`, the `index`th; `function_at` is the
/// index of an aggregate function's function argument, after an optional
/// version number.
fn type_arg(p: &mut Parser<'_>, name: &str, index: usize, function_at: &mut usize) -> Option<()> {
    match name {
        "Nested" => name_type_pair(p).map(drop),
        "Tuple" if names::at_name(p, 0) && at_data_type(p, 1) => name_type_pair(p).map(drop),
        "Tuple" => data_type(p).map(drop),
        "Dynamic" => exprs::expr(p).map(drop),
        "AggregateFunction" | "SimpleAggregateFunction" => {
            if index == 0 && name == "AggregateFunction" && p.at(T::Number) {
                *function_at = 1;
                exprs::literal(p).map(drop)
            } else if index == *function_at {
                exprs::expr(p).map(drop)
            } else {
                data_type(p).map(drop)
            }
        }
        _ if name.eq_ignore_ascii_case("JSON") => json_arg(p),
        _ if exprs::at_literal(p, 0) || (names::at_name(p, 0) && p.nth_at(1, T::Equals)) => {
            // Literals, `'a' = 1` of an `Enum`, and `name = value`
            // parameters.
            exprs::expr(p).map(drop)
        }
        _ => data_type(p).map(drop),
    }
}

/// `name Type`, where the name may be dotted (`JSON(a.b UInt32)`).
fn name_type_pair(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    names::compound_name(p)?;
    data_type(p)?;
    Some(m.complete(p, SyntaxKind::NameTypePair))
}

/// An argument of `JSON(…)`: `SKIP path`, `SKIP REGEXP '…'`,
/// `parameter = n`, or `path Type`.
fn json_arg(p: &mut Parser<'_>) -> Option<()> {
    if p.at_kw(K::Skip) && (names::at_name(p, 1) || p.nth_at_kw(1, K::Regexp)) {
        let m = p.start();
        p.bump_keyword();
        if p.at_kw(K::Regexp) && p.nth_at(1, T::StringLiteral) {
            p.bump_keyword();
            exprs::literal(p)?;
        } else {
            names::compound_name(p)?;
        }
        m.complete(p, SyntaxKind::JsonSkip);
        return Some(());
    }
    if names::at_name(p, 0) && p.nth_at(1, T::Equals) {
        return exprs::expr(p).map(drop);
    }
    name_type_pair(p).map(drop)
}

#[cfg(test)]
mod tests;
