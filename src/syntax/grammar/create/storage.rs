//! The storage clauses of a table or view (`ParserStorage`): `ENGINE`, the
//! keys, `ORDER BY` and `TTL`. `SETTINGS` is
//! [`common::settings_clause`](crate::syntax::grammar::common::settings_clause).

use super::elements::codec_node;
use super::name_with_args;
use super::string;
use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::exprs::AliasMode;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::fallback::Stop;
use crate::syntax::grammar::names;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Whether the current token starts a storage clause: `ENGINE`, `PARTITION
/// BY`, `PRIMARY KEY`, `ORDER BY`, `UNIQUE KEY`, `SAMPLE BY`, `TTL` or
/// `SETTINGS`. `engine = 1` and `ttl + 1` do not.
pub(super) fn at_storage_clause(p: &Parser<'_>) -> bool {
    match p.nth_keyword(0) {
        Some(K::Engine) => p.nth_at(1, T::Equals) || names::at_name(p, 1),
        Some(K::Partition | K::Order | K::Sample) => p.nth_at_kw(1, K::By),
        Some(K::Primary | K::Unique) => p.nth_at_kw(1, K::Key),
        Some(K::Ttl) => super::at_operand(p, 1),
        Some(K::Settings) => common::at_settings(p),
        _ => false,
    }
}

/// The storage clause at the current token, whose lists end at `ends`.
pub(super) fn storage_clause(p: &mut Parser<'_>, ends: Stop<'_>) -> Option<()> {
    match p.nth_keyword(0) {
        Some(K::Engine) => engine(p),
        Some(K::Partition) => key(p, SyntaxKind::PartitionByClause),
        Some(K::Primary) => key(p, SyntaxKind::PrimaryKeyClause),
        Some(K::Unique) => key(p, SyntaxKind::UniqueKeyClause),
        Some(K::Sample) => key(p, SyntaxKind::SampleByClause),
        Some(K::Order) => order_by(p),
        Some(K::Ttl) => ttl(p, ends),
        Some(K::Settings) => common::settings_clause(p, ends).map(drop),
        _ => p.expected("a storage clause"),
    }
}

/// If the current token starts a target of a `TimeSeries` table
/// (`ParserViewTargets`), how many keywords name the target: `SAMPLES`,
/// `DATA`, `RECENT SAMPLES`, `TAGS`, `TIME RANGES`, `METRIC FAMILIES` or
/// `METRICS`, followed by `INNER`, `ENGINE` or a table name.
pub(super) fn at_view_target(p: &Parser<'_>) -> Option<usize> {
    let words = match p.nth_keyword(0)? {
        K::Samples | K::Data | K::Tags | K::Metrics => 1,
        K::Recent if p.nth_at_kw(1, K::Samples) => 2,
        K::Time if p.nth_at_kw(1, K::Ranges) => 2,
        K::Metric if p.nth_at_kw(1, K::Families) => 2,
        _ => return None,
    };
    names::at_name(p, words).then_some(words)
}

/// A target of a `TimeSeries` table as a
/// [`ViewTargetClause`](SyntaxKind::ViewTargetClause): its keywords, then
/// `INNER UUID '…'`, `INNER COLUMNS (…)`, `[INNER]` and the storage
/// clauses of the target table, or the target table's name.
pub(super) fn view_target(p: &mut Parser<'_>, ends: Stop<'_>) -> Option<()> {
    let words = at_view_target(p).unwrap_or(1);
    let m = p.start();
    for _ in 0..words {
        p.bump_keyword();
    }
    if p.at_kws(&[K::Inner, K::Uuid]) && p.nth_at(2, T::StringLiteral) {
        p.bump_keyword();
        p.bump_keyword();
        string(p)?;
    } else if p.at_kws(&[K::Inner, K::Columns]) && p.nth_at(2, T::OpeningRoundBracket) {
        p.bump_keyword();
        p.bump_keyword();
        super::elements::element_list(p)?;
    } else if p.at_kw(K::Inner) || at_storage_clause(p) {
        p.eat_kw(K::Inner);
        if !at_storage_clause(p) {
            return p.expected("ENGINE or a storage clause");
        }
        while at_storage_clause(p) {
            storage_clause(p, ends)?;
        }
    } else {
        names::compound_name(p)?;
    }
    m.complete(p, SyntaxKind::ViewTargetClause);
    Some(())
}

/// `ENGINE [=] name[(args)]` as an [`EngineClause`](SyntaxKind::EngineClause).
/// The engine is a name, never a keyword or an expression: `ENGINE = Null`.
pub(super) fn engine(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.eat(T::Equals);
    name_with_args(p)?;
    m.complete(p, SyntaxKind::EngineClause);
    Some(())
}

/// Two keywords and an expression, as a clause of `kind`: `PARTITION BY`,
/// `PRIMARY KEY`, `UNIQUE KEY`, `SAMPLE BY`.
pub(crate) fn key(p: &mut Parser<'_>, kind: SyntaxKind) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.bump_keyword();
    exprs::expr(p)?;
    m.complete(p, kind);
    Some(())
}

/// `ORDER BY` of a table, as an [`OrderByClause`](SyntaxKind::OrderByClause)
/// (`ParserStorageOrderByClause`): one expression with an optional
/// direction, which covers `ORDER BY (a, b)` and `ORDER BY tuple()`, or else
/// keys with directions in `(…)` or `tuple(…)`, an
/// [`OrderByTuple`](SyntaxKind::OrderByTuple).
pub(crate) fn order_by(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    p.bump_keyword();
    if at_order_by_tuple(p) {
        order_by_tuple(p)?;
    } else {
        let key = exprs::expr(p)?;
        if is_direction(p, 0) {
            let item = key.precede(p);
            p.bump_keyword();
            item.complete(p, SyntaxKind::OrderByItem);
        }
    }
    m.complete(p, SyntaxKind::OrderByClause);
    Some(())
}

/// Whether the `n`th token is `ASC`, `DESC`, `ASCENDING` or `DESCENDING`.
fn is_direction(p: &Parser<'_>, n: usize) -> bool {
    matches!(
        p.nth_keyword(n),
        Some(K::Asc | K::Desc | K::Ascending | K::Descending)
    )
}

/// Whether a table's `ORDER BY` keys at the current token are `(…)` or
/// `tuple(…)` with a direction on a key, `(a DESC, b)`: an expression
/// cannot hold it, so ClickHouse reads keys with directions instead. A
/// direction there follows an operand and precedes `,` or `)`, which a
/// column called `desc` does not.
fn at_order_by_tuple(p: &Parser<'_>) -> bool {
    let open = usize::from(p.nth_text(0) == "tuple" && p.nth_at(1, T::OpeningRoundBracket));
    p.nth_at(open, T::OpeningRoundBracket)
        && p.group_items(open).any(|n| {
            is_direction(p, n)
                && !matches!(p.nth(n - 1), Some(T::OpeningRoundBracket | T::Comma))
                && matches!(p.nth(n + 1), Some(T::Comma | T::ClosingRoundBracket))
        })
}

/// `(a DESC, b)` or `tuple(a DESC, b)`. ClickHouse takes `tuple` here only
/// in lower case.
fn order_by_tuple(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    if p.nth_text(0) == "tuple" && p.nth_at(1, T::OpeningRoundBracket) {
        names::name(p)?;
    }
    p.expect(T::OpeningRoundBracket)?;
    if !p.at(T::ClosingRoundBracket) {
        common::order_by_list(p, &fallback::never)?;
    }
    p.expect(T::ClosingRoundBracket)?;
    Some(m.complete(p, SyntaxKind::OrderByTuple))
}

/// `TTL rule, …` of a table, as a [`TtlClause`](SyntaxKind::TtlClause) with
/// a [`TtlList`](SyntaxKind::TtlList) that ends at `ends`.
pub(crate) fn ttl(p: &mut Parser<'_>, ends: Stop<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    let list = p.start();
    let shape = fallback::comma_separated(p, ends, &mut |p| ttl_element(p, ends).map(drop));
    if shape.items == 0 && shape.commas == 0 {
        list.abandon(p);
        return p.expected("a TTL rule");
    }
    list.complete(p, SyntaxKind::TtlList);
    m.complete(p, SyntaxKind::TtlClause);
    Some(())
}

/// One rule of a table's `TTL`, a [`TtlElement`](SyntaxKind::TtlElement)
/// (`ParserTTLElement`): an expression, then `TO DISK|VOLUME [IF EXISTS]
/// '…'`, `GROUP BY keys [SET assignments]`, `RECOMPRESS CODEC(…)`, or
/// `[DELETE] [WHERE expr]`.
fn ttl_element(p: &mut Parser<'_>, ends: Stop<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    exprs::expr(p)?;
    if p.at_kw(K::To) && matches!(p.nth_keyword(1), Some(K::Disk | K::Volume)) {
        p.bump_keyword();
        p.bump_keyword();
        common::if_exists(p);
        string(p)?;
    } else if p.at_kws(&[K::Group, K::By]) {
        p.bump_keyword();
        p.bump_keyword();
        common::expr_list(
            p,
            &|p: &Parser<'_>| p.at_kw(K::Set) || ends(p),
            AliasMode::As,
        )?;
        if p.eat_kw(K::Set) {
            assignments(p)?;
        }
    } else if p.at_kw(K::Recompress) {
        p.bump_keyword();
        if !p.at_kw(K::Codec) || !p.nth_at(1, T::OpeningRoundBracket) {
            return p.expected("CODEC");
        }
        codec_node(p, SyntaxKind::ColumnCodec)?;
    } else {
        p.eat_kw(K::Delete);
        if p.eat_kw(K::Where) {
            exprs::expr(p)?;
        }
    }
    Some(m.complete(p, SyntaxKind::TtlElement))
}

/// `a = expr, b = expr` as an [`AssignmentList`](SyntaxKind::AssignmentList).
/// Like ClickHouse's list parser it stops before a comma that is not
/// followed by another assignment, which then separates TTL rules.
fn assignments(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    assignment(p)?;
    while p.at(T::Comma) && names::at_name(p, 1) && p.nth_at(2, T::Equals) {
        p.bump();
        assignment(p)?;
    }
    Some(m.complete(p, SyntaxKind::AssignmentList))
}

/// `name = expr` as an [`Assignment`](SyntaxKind::Assignment).
pub(crate) fn assignment(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    names::name(p)?;
    p.expect(T::Equals)?;
    exprs::expr(p)?;
    Some(m.complete(p, SyntaxKind::Assignment))
}
