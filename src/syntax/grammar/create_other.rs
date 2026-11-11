//! `CREATE DICTIONARY`, `CREATE FUNCTION` and `CREATE INDEX`, as
//! ClickHouse's `ParserCreateDictionaryQuery` (with `ParserDictionary` and
//! `ParserDictionaryAttributeDeclaration`), `ParserCreateFunctionQuery` and
//! `ParserCreateIndexQuery` read them.
//!
//! A dictionary is a [`CreateHeader`](SyntaxKind::CreateHeader), its
//! attributes in brackets, then clauses in any order, each with the clause
//! fallback: `PRIMARY KEY`, `SOURCE(…)`, `LAYOUT(…)`, `LIFETIME(…)`,
//! `RANGE(…)`, `SETTINGS(…)` and `COMMENT`.
//!
//! # Keyword slots
//!
//! Inside `SOURCE(…)`, `LAYOUT(…)`, `LIFETIME(…)` and `RANGE(…)` every word
//! is a name: ClickHouse reads the source or layout and each key with
//! `ParserIdentifier`, whatever they spell, and prints them in upper case
//! itself. So `SOURCE(clickhouse(table 'x' db 'y'))` keeps its case, as do
//! `MIN` and `MAX` in `LIFETIME(MIN 0 MAX 10)`. A function's name and its
//! lambda's parameters are names too.
//!
//! An index's name, its table and its type are names: `TYPE minmax`,
//! `TYPE bloom_filter(0.01)`.

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::create;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::insert;
use crate::syntax::grammar::names;
use crate::syntax::grammar::types;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Whether the statement at the current token is `CREATE [OR REPLACE]
/// DICTIONARY`, `CREATE [OR REPLACE] FUNCTION` or `CREATE [UNIQUE] INDEX`.
pub(crate) fn at_statement(p: &Parser<'_>) -> bool {
    if !p.at_kw(K::Create) {
        return false;
    }
    if at_index(p) {
        return true;
    }
    let n = if p.nth_at_kw(1, K::Or) && p.nth_at_kw(2, K::Replace) {
        3
    } else {
        1
    };
    matches!(p.nth_keyword(n), Some(K::Dictionary | K::Function))
}

/// A statement of this module's, up to the end of the statement.
pub(crate) fn statement(p: &mut Parser<'_>) -> Option<()> {
    if at_index(p) {
        index(p)
    } else if p.at_kws(&[K::Create, K::Dictionary])
        || p.at_kws(&[K::Create, K::Or, K::Replace, K::Dictionary])
    {
        dictionary(p)
    } else {
        function(p)
    }
}

/// A [`CreateDictionary`](SyntaxKind::CreateDictionary).
fn dictionary(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    create::header(p, &[K::Dictionary])?;
    fallback::delimited(
        p,
        SyntaxKind::DictionaryAttributeList,
        T::OpeningRoundBracket,
        T::ClosingRoundBracket,
        &mut |p| attribute(p).map(drop),
    )?;
    while !p.at_end() {
        fallback::clause(p, &at_clause, clause);
    }
    m.complete(p, SyntaxKind::CreateDictionary);
    Some(())
}

/// Whether the current token starts a clause of a dictionary.
fn at_clause(p: &Parser<'_>) -> bool {
    match p.nth_keyword(0) {
        Some(K::Primary) => p.nth_at_kw(1, K::Key),
        Some(K::Source | K::Layout | K::Lifetime | K::Range | K::Settings) => {
            p.nth_at(1, T::OpeningRoundBracket)
        }
        Some(K::Comment) => p.nth_at(1, T::StringLiteral),
        _ => false,
    }
}

/// The clause of a dictionary at the current token.
fn clause(p: &mut Parser<'_>) -> Option<()> {
    let keyword = p.nth_keyword(0);
    if keyword == Some(K::Comment) {
        return create::comment(p);
    }
    if keyword == Some(K::Primary) {
        return primary_key(p);
    }
    let m = p.start();
    p.bump_keyword();
    p.expect(T::OpeningRoundBracket)?;
    match keyword {
        Some(K::Source) => {
            key_value_function(p, false)?;
        }
        Some(K::Layout) => {
            key_value_function(p, true)?;
        }
        Some(K::Lifetime) if p.at(T::Number) => {
            exprs::literal(p)?;
        }
        Some(K::Lifetime | K::Range) => {
            pairs(p)?;
        }
        Some(K::Settings) => {
            common::settings_list(p, &fallback::never)?;
        }
        _ => return p.expected("a clause"),
    }
    p.expect(T::ClosingRoundBracket)?;
    m.complete(p, SyntaxKind::DictionaryClause);
    Some(())
}

/// `PRIMARY KEY` and the key attributes, in brackets or not, as a
/// [`DictionaryPrimaryKey`](SyntaxKind::DictionaryPrimaryKey).
fn primary_key(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.bump_keyword();
    let bracketed = p.eat(T::OpeningRoundBracket);
    let list = p.start();
    let shape = fallback::comma_separated(p, &at_clause, &mut |p| names::name(p).map(drop));
    if shape.items == 0 {
        list.abandon(p);
        return p.expected("a key attribute");
    }
    list.complete(p, SyntaxKind::NameList);
    if bracketed {
        p.expect(T::ClosingRoundBracket)?;
    }
    m.complete(p, SyntaxKind::DictionaryPrimaryKey);
    Some(())
}

/// A [`DictionaryAttribute`](SyntaxKind::DictionaryAttribute): a name, a
/// type, and its properties in any order, each at most once.
fn attribute(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    names::name(p)?;
    types::data_type(p)?;
    let mut seen: Vec<K> = Vec::new();
    while let Some(keyword) = p.nth_keyword(0) {
        if seen.contains(&keyword) {
            break;
        }
        match keyword {
            K::Default => {
                p.bump_keyword();
                value(p)?;
            }
            K::Expression => {
                p.bump_keyword();
                exprs::expr(p)?;
            }
            K::Hierarchical | K::Bidirectional | K::Injective | K::IsObjectId => p.bump_keyword(),
            _ => break,
        }
        seen.push(keyword);
    }
    Some(m.complete(p, SyntaxKind::DictionaryAttribute))
}

/// A literal, a signed number or an array of literals: an attribute's
/// default (`ParserLiteral`, `ParserArrayOfLiterals`).
fn value(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if p.at(T::OpeningSquareBracket) || p.at(T::Minus) || p.at(T::Plus) {
        exprs::expr(p)
    } else {
        exprs::literal(p)
    }
}

/// `name(pairs)` as a [`KeyValueFunction`](SyntaxKind::KeyValueFunction)
/// (`ParserFunctionWithKeyValueArguments`); with `bare`, as in `LAYOUT`, the
/// name alone may stand for `name()`.
fn key_value_function(p: &mut Parser<'_>, bare: bool) -> Option<CompletedMarker> {
    let m = p.start();
    names::name(p)?;
    if p.eat(T::OpeningRoundBracket) {
        if !p.at(T::ClosingRoundBracket) {
            pairs(p)?;
        }
        p.expect(T::ClosingRoundBracket)?;
    } else if !bare {
        return p.expected("`(`");
    }
    Some(m.complete(p, SyntaxKind::KeyValueFunction))
}

/// [`KeyValuePairs`](SyntaxKind::KeyValuePairs): pairs one after another
/// for as long as a name starts one. Fails if there is none.
fn pairs(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let mut count = 0;
    while names::at_name(p, 0) {
        pair(p)?;
        count += 1;
    }
    if count == 0 {
        m.abandon(p);
        return p.expected("a key and a value");
    }
    Some(m.complete(p, SyntaxKind::KeyValuePairs))
}

/// A [`KeyValuePair`](SyntaxKind::KeyValuePair) (`ParserKeyValuePair`):
/// a name, then a function call, a literal, an array of literals, a name,
/// or pairs in brackets, tried in that order. A Jinja expression before the
/// closing bracket stands for pairs, and has no value.
fn pair(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let jinja = p.at(T::JinjaExpr);
    names::name(p)?;
    if jinja && p.at(T::ClosingRoundBracket) {
        return Some(m.complete(p, SyntaxKind::KeyValuePair));
    }
    if p.eat(T::OpeningRoundBracket) {
        pairs(p)?;
        p.expect(T::ClosingRoundBracket)?;
    } else if names::at_name(p, 0) && p.nth_at(1, T::OpeningRoundBracket) {
        exprs::expr(p)?;
    } else if exprs::at_literal(p, 0) || p.at(T::OpeningSquareBracket) {
        value(p)?;
    } else {
        names::name(p)?;
    }
    Some(m.complete(p, SyntaxKind::KeyValuePair))
}

/// A [`CreateFunction`](SyntaxKind::CreateFunction): the head, then `AS` and
/// an expression, or what follows kept as written.
fn function(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.eat_kws(&[K::Or, K::Replace]);
    p.expect_kw(K::Function)?;
    common::if_not_exists(p);
    names::name(p)?;
    if p.at_kw(K::On) {
        common::on_cluster(p)?;
    }
    if p.eat_kw(K::As) {
        exprs::expr(p)?;
    } else {
        fallback::opaque_until(p, &fallback::never)?;
    }
    m.complete(p, SyntaxKind::CreateFunction);
    Some(())
}

/// Whether the statement at the current token is `CREATE [UNIQUE] INDEX`.
fn at_index(p: &Parser<'_>) -> bool {
    p.at_kws(&[K::Create, K::Index]) || p.at_kws(&[K::Create, K::Unique, K::Index])
}

/// A [`CreateIndex`](SyntaxKind::CreateIndex) (`ParserCreateIndexQuery`,
/// `ParserCreateIndexDeclaration`). In brackets, the indexed expressions
/// are `ORDER BY` items, whose directions ClickHouse accepts and ignores;
/// without, one expression.
fn index(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.eat_kw(K::Unique);
    p.expect_kw(K::Index)?;
    common::if_not_exists(p);
    names::name(p)?;
    p.expect_kw(K::On)?;
    insert::table_name(p)?;
    if p.at_kw(K::On) {
        common::on_cluster(p)?;
    }
    if p.at(T::OpeningRoundBracket) {
        let tuple = p.start();
        p.bump();
        common::order_by_list(p, &fallback::never)?;
        p.expect(T::ClosingRoundBracket)?;
        tuple.complete(p, SyntaxKind::OrderByTuple);
    } else {
        exprs::expr(p)?;
    }
    if p.eat_kw(K::Type) {
        create::name_with_args(p)?;
    }
    if p.eat_kw(K::Granularity) {
        if !p.at(T::Number) {
            return p.expected("a number");
        }
        exprs::literal(p)?;
    }
    m.complete(p, SyntaxKind::CreateIndex);
    Some(())
}

#[cfg(test)]
mod tests;
