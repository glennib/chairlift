//! The element list of `CREATE TABLE` (`ParserTablePropertiesDeclarationList`):
//! columns, indexes, projections, constraints, and keys.

use super::name_with_args;
use super::string;
use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::exprs::AliasMode;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::names;
use crate::syntax::grammar::select;
use crate::syntax::grammar::types;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// `( element, … )` as a [`TableElementList`](SyntaxKind::TableElementList),
/// each element with the item fallback. A trailing comma is accepted, as
/// ClickHouse does.
pub(super) fn element_list(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    fallback::delimited(
        p,
        SyntaxKind::TableElementList,
        T::OpeningRoundBracket,
        T::ClosingRoundBracket,
        &mut |p| element(p).map(drop),
    )
}

/// One element (`ParserTablePropertyDeclaration`). `INDEX`, `CONSTRAINT`,
/// `PROJECTION`, `PRIMARY KEY` and `FOREIGN KEY` always start the element
/// they name, so ClickHouse rejects a column called `index` unless it is
/// quoted, and so does this.
fn element(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    match p.nth_keyword(0) {
        Some(K::Index) => index(p),
        Some(K::Constraint) => constraint(p),
        Some(K::Projection) => projection(p),
        Some(K::Primary) if p.nth_at_kw(1, K::Key) => {
            let m = p.start();
            p.bump_keyword();
            p.bump_keyword();
            exprs::expr(p)?;
            Some(m.complete(p, SyntaxKind::TablePrimaryKey))
        }
        Some(K::Foreign) if p.nth_at_kw(1, K::Key) => foreign_key(p),
        _ => column(p),
    }
}

/// `INDEX name expr TYPE type [GRANULARITY n]` (`ParserIndexDeclaration`).
fn index(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.bump_keyword();
    names::name(p)?;
    exprs::expr(p)?;
    p.expect_kw(K::Type)?;
    name_with_args(p)?;
    if p.eat_kw(K::Granularity) {
        number(p)?;
    }
    Some(m.complete(p, SyntaxKind::IndexDef))
}

/// A number literal.
fn number(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !p.at(T::Number) {
        return p.expected("a number");
    }
    exprs::literal(p)
}

/// `CONSTRAINT name CHECK|ASSUME expr` (`ParserConstraintDeclaration`).
fn constraint(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.bump_keyword();
    names::name(p)?;
    if !p.eat_kw(K::Check) && !p.eat_kw(K::Assume) {
        return p.expected("CHECK or ASSUME");
    }
    exprs::expr(p)?;
    Some(m.complete(p, SyntaxKind::ConstraintDef))
}

/// `PROJECTION name (query)` or `PROJECTION name INDEX exprs TYPE type`,
/// then `WITH SETTINGS (…)` (`parseProjectionDeclarationBody`). The query
/// is ClickHouse's projection form of `SELECT`, which the query grammar
/// covers.
fn projection(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.bump_keyword();
    names::name(p)?;
    if p.at(T::OpeningRoundBracket) {
        select::subquery(p)?;
    } else if p.eat_kw(K::Index) {
        common::expr_list(p, &|p: &Parser<'_>| p.at_kw(K::Type), AliasMode::As)?;
        p.expect_kw(K::Type)?;
        name_with_args(p)?;
    } else {
        return p.expected("`(` or INDEX");
    }
    if p.at_kws(&[K::With, K::Settings]) && p.nth_at(2, T::OpeningRoundBracket) {
        let s = p.start();
        p.bump_keyword();
        settings_in_brackets(p)?;
        s.complete(p, SyntaxKind::ProjectionSettings);
    }
    Some(m.complete(p, SyntaxKind::ProjectionDef))
}

/// `SETTINGS (a = 1, …)`: the keyword and the bracketed list.
fn settings_in_brackets(p: &mut Parser<'_>) -> Option<()> {
    p.expect_kw(K::Settings)?;
    p.expect(T::OpeningRoundBracket)?;
    common::settings_list(p, &fallback::never)?;
    p.expect(T::ClosingRoundBracket)
}

/// `FOREIGN KEY expr REFERENCES table expr [ON DELETE|UPDATE action]…`
/// (`ParserForeignKeyDeclaration`).
fn foreign_key(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.bump_keyword();
    p.bump_keyword();
    exprs::expr(p)?;
    p.expect_kw(K::References)?;
    names::compound_name(p)?;
    exprs::expr(p)?;
    while p.at_kw(K::On) && matches!(p.nth_keyword(1), Some(K::Delete | K::Update)) {
        p.bump_keyword();
        p.bump_keyword();
        let action: &[K] = match p.nth_keyword(0) {
            Some(K::Restrict) => &[K::Restrict],
            Some(K::Cascade) => &[K::Cascade],
            Some(K::Set) if p.nth_at_kw(1, K::Null) => &[K::Set, K::Null],
            Some(K::Set) if p.nth_at_kw(1, K::Default) => &[K::Set, K::Default],
            Some(K::No) if p.nth_at_kw(1, K::Action) => &[K::No, K::Action],
            _ => return p.expected("RESTRICT, CASCADE, SET NULL, NO ACTION or SET DEFAULT"),
        };
        p.eat_kws(action);
    }
    Some(m.complete(p, SyntaxKind::ForeignKeyDef))
}

/// Whether the current tokens are `NULL` or `NOT NULL`.
fn at_null(p: &Parser<'_>) -> bool {
    p.at_kw(K::Null) || p.at_kws(&[K::Not, K::Null])
}

/// A [`ColumnDef`](SyntaxKind::ColumnDef), as `IParserColumnDeclaration`
/// reads one in `CREATE TABLE`: the name, the type unless a default or
/// modifier keyword comes first, `NULL`/`NOT NULL` and `COLLATE` around the
/// default, then the other modifiers in any order, each at most once.
fn column(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    names::name(p)?;
    let typeless = at_null(p)
        || matches!(
            p.nth_keyword(0),
            Some(
                K::Default | K::Materialized | K::Ephemeral | K::Alias | K::AutoIncrement | K::Ttl
            )
        )
        || p.at_kws(&[K::Primary, K::Key]);
    let mut collate = false;
    if !typeless {
        types::data_type(p)?;
        if p.at_kw(K::Collate) {
            column_collate(p)?;
            collate = true;
        }
    }
    let null = column_null(p).is_some();
    if !collate && p.at_kw(K::Collate) {
        column_collate(p)?;
        collate = true;
    }
    // Whether something stands in for the type, and whether `NULL` may
    // still follow.
    let (defaulted, null_after) = match p.nth_keyword(0) {
        Some(K::Default | K::Materialized | K::Alias) => {
            let d = p.start();
            p.bump_keyword();
            exprs::expr(p)?;
            d.complete(p, SyntaxKind::ColumnDefault);
            (true, true)
        }
        Some(K::Ephemeral) => {
            let d = p.start();
            p.bump_keyword();
            // After `EPHEMERAL`, `COMMENT` starts the column's comment;
            // anything else but the end of the column is a default
            // expression, even a modifier's keyword: ClickHouse reads
            // `EPHEMERAL CODEC(ZSTD)` as the default `CODEC(ZSTD)`.
            let comment = p.at_kw(K::Comment);
            let default =
                !comment && !matches!(p.nth(0), None | Some(T::Comma | T::ClosingRoundBracket));
            if default {
                exprs::expr(p)?;
            }
            d.complete(p, SyntaxKind::ColumnDefault);
            (default, !comment)
        }
        Some(K::AutoIncrement) => {
            let d = p.start();
            p.bump_keyword();
            d.complete(p, SyntaxKind::ColumnDefault);
            (true, true)
        }
        _ => (false, true),
    };
    if typeless && !defaulted {
        return p.expected("a data type or a default");
    }
    if !null && null_after {
        column_null(p);
    }
    modifiers(p, collate)?;
    Some(m.complete(p, SyntaxKind::ColumnDef))
}

/// `NULL` or `NOT NULL`, if it follows.
fn column_null(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !at_null(p) {
        return None;
    }
    let m = p.start();
    p.eat_kw(K::Not);
    p.bump_keyword();
    Some(m.complete(p, SyntaxKind::ColumnNull))
}

/// `COLLATE name`.
fn column_collate(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.bump_keyword();
    names::name(p)?;
    Some(m.complete(p, SyntaxKind::ColumnCollate))
}

/// The modifiers after a column's default, in any order, each at most once:
/// `COMMENT`, `CODEC`, `STATISTICS`, `TTL`, `COLLATE` (unless `collate`
/// says the column has one), `PRIMARY KEY`, `SETTINGS (…)`.
fn modifiers(p: &mut Parser<'_>, mut collate: bool) -> Option<()> {
    let mut comment = false;
    let mut codec = false;
    let mut statistics = false;
    let mut ttl = false;
    let mut primary_key = false;
    let mut settings = false;
    loop {
        if !comment && p.at_kw(K::Comment) && p.nth_at(1, T::StringLiteral) {
            comment = true;
            let m = p.start();
            p.bump_keyword();
            string(p)?;
            m.complete(p, SyntaxKind::ColumnComment);
        } else if !codec && p.at_kw(K::Codec) && p.nth_at(1, T::OpeningRoundBracket) {
            codec = true;
            codec_node(p, SyntaxKind::ColumnCodec)?;
        } else if !statistics && p.at_kw(K::Statistics) && p.nth_at(1, T::OpeningRoundBracket) {
            statistics = true;
            codec_node(p, SyntaxKind::ColumnStatistics)?;
        } else if !ttl && p.at_kw(K::Ttl) {
            ttl = true;
            let m = p.start();
            p.bump_keyword();
            exprs::expr(p)?;
            m.complete(p, SyntaxKind::ColumnTtl);
        } else if !collate && p.at_kw(K::Collate) {
            collate = true;
            column_collate(p)?;
        } else if !primary_key && p.at_kws(&[K::Primary, K::Key]) {
            primary_key = true;
            let m = p.start();
            p.bump_keyword();
            p.bump_keyword();
            m.complete(p, SyntaxKind::ColumnPrimaryKey);
        } else if !settings && p.at_kw(K::Settings) && p.nth_at(1, T::OpeningRoundBracket) {
            settings = true;
            let m = p.start();
            settings_in_brackets(p)?;
            m.complete(p, SyntaxKind::ColumnSettings);
        } else {
            return Some(());
        }
    }
}

/// `CODEC(…)` or `STATISTICS(…)` as a node of `kind`: the keyword and a
/// [`CodecList`](SyntaxKind::CodecList) (`ParserCodec`,
/// `ParserStatisticsType`).
pub(super) fn codec_node(p: &mut Parser<'_>, kind: SyntaxKind) -> Option<CompletedMarker> {
    let m = p.start();
    p.bump_keyword();
    fallback::delimited(
        p,
        SyntaxKind::CodecList,
        T::OpeningRoundBracket,
        T::ClosingRoundBracket,
        &mut |p| name_with_args(p).map(drop),
    )?;
    Some(m.complete(p, kind))
}
