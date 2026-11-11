//! Grammar that several statements share: expression lists, `ORDER BY`
//! items, `SETTINGS`, `ON CLUSTER`, `IF [NOT] EXISTS`.

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::exprs::AliasMode;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::fallback::Stop;
use crate::syntax::grammar::names;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Expressions separated by commas, each with an alias as `alias` allows and
/// with item fallback, as an [`ExprList`](SyntaxKind::ExprList), with items
/// that end at `ends` as [`fallback::comma_separated`] reads them. Fails if
/// the list is empty.
pub(crate) fn expr_list(
    p: &mut Parser<'_>,
    ends: Stop<'_>,
    alias: AliasMode,
) -> Option<CompletedMarker> {
    let m = p.start();
    let shape =
        fallback::comma_separated(p, ends, &mut |p| exprs::expr_with_alias(p, alias).map(drop));
    if shape.items == 0 && shape.commas == 0 {
        m.abandon(p);
        return p.expected("an expression");
    }
    Some(m.complete(p, SyntaxKind::ExprList))
}

/// `ORDER BY` items separated by commas, as an
/// [`OrderByList`](SyntaxKind::OrderByList), with items that end at `ends`.
/// Fails if the list is empty.
pub(crate) fn order_by_list(p: &mut Parser<'_>, ends: Stop<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let shape = fallback::comma_separated(p, ends, &mut |p| order_by_item(p).map(drop));
    if shape.items == 0 && shape.commas == 0 {
        m.abandon(p);
        return p.expected("an ORDER BY item");
    }
    Some(m.complete(p, SyntaxKind::OrderByList))
}

/// An [`OrderByItem`](SyntaxKind::OrderByItem) (`ParserOrderByElement`):
/// an expression with an optional `AS` alias, then any of `ASC`/`DESC`,
/// `NULLS FIRST|LAST`, `COLLATE '…'` and `WITH FILL [FROM e] [TO e] [STEP e]
/// [STALENESS e]`, in that order.
pub(crate) fn order_by_item(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    exprs::expr_with_alias(p, AliasMode::As)?;
    if matches!(
        p.nth_keyword(0),
        Some(K::Asc | K::Desc | K::Ascending | K::Descending)
    ) {
        p.bump_keyword();
    }
    if p.at_kw(K::Nulls) && matches!(p.nth_keyword(1), Some(K::First | K::Last)) {
        p.bump_keyword();
        p.bump_keyword();
    }
    if p.at_kw(K::Collate) && p.nth_at(1, T::StringLiteral) {
        p.bump_keyword();
        exprs::literal(p)?;
    }
    if p.eat_kws(&[K::With, K::Fill]) {
        for keyword in [K::From, K::To, K::Step, K::Staleness] {
            if p.eat_kw(keyword) {
                exprs::expr(p)?;
            }
        }
    }
    Some(m.complete(p, SyntaxKind::OrderByItem))
}

/// Whether `SETTINGS` at the current token starts a settings clause: where
/// a clause may start, ClickHouse reads `SETTINGS` as one whenever a
/// setting's name follows it (`ParserSetQuery`), with or without `=`, so
/// `FROM t SETTINGS final` sets `final`. Before `=`, `(` or the end it is a
/// name.
pub(crate) fn at_settings(p: &Parser<'_>) -> bool {
    p.at_kw(K::Settings) && names::at_name(p, 1)
}

/// `SETTINGS a = 1, …` as a [`SettingsClause`](SyntaxKind::SettingsClause),
/// with items that end at `ends`.
pub(crate) fn settings_clause(p: &mut Parser<'_>, ends: Stop<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect_kw(K::Settings)?;
    settings_list(p, ends)?;
    Some(m.complete(p, SyntaxKind::SettingsClause))
}

/// `a = 1, b = 'x', c = DEFAULT, d` as a
/// [`SettingsList`](SyntaxKind::SettingsList), with items that end at
/// `ends`. Values are expressions, or maps of strings (`{'k': 'v'}`); a
/// name without a value is ClickHouse's shorthand for `= true`. Setting
/// names keep their case.
pub(crate) fn settings_list(p: &mut Parser<'_>, ends: Stop<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let shape = fallback::comma_separated(p, ends, &mut |p| {
        let item = p.start();
        names::compound_name(p)?;
        if !p.eat(T::Equals) {
            item.complete(p, SyntaxKind::SettingItem);
            return Some(());
        }
        if p.at_kw(K::Default) && (p.nth_at(1, T::Comma) || p.nth(1).is_none() || ends_at(p, 1)) {
            p.bump_keyword();
        } else if p.at(T::OpeningCurlyBrace) && !names::at_query_parameter(p, 0) {
            exprs::map_literal(p, &mut |p| exprs::string(p).map(drop))?;
        } else {
            exprs::expr(p)?;
        }
        item.complete(p, SyntaxKind::SettingItem);
        Some(())
    });
    if shape.items == 0 && shape.commas == 0 {
        m.abandon(p);
        return p.expected("a setting");
    }
    Some(m.complete(p, SyntaxKind::SettingsList))
}

/// Whether the `n`th token closes a bracket.
fn ends_at(p: &Parser<'_>, n: usize) -> bool {
    p.nth(n).is_some_and(fallback::is_closer)
}

/// `ON CLUSTER name` or `ON CLUSTER 'name'`, if it follows.
pub(crate) fn on_cluster(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !p.at_kws(&[K::On, K::Cluster]) {
        return None;
    }
    let m = p.start();
    p.bump_keyword();
    p.bump_keyword();
    if p.at(T::StringLiteral) {
        exprs::literal(p)?;
    } else {
        names::name(p)?;
    }
    Some(m.complete(p, SyntaxKind::OnCluster))
}

/// `IF NOT EXISTS`, if it follows.
pub(crate) fn if_not_exists(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    keywords(p, &[K::If, K::Not, K::Exists], SyntaxKind::IfNotExists)
}

/// `IF EXISTS`, if it follows.
pub(crate) fn if_exists(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    keywords(p, &[K::If, K::Exists], SyntaxKind::IfExists)
}

/// `words` as a node of `kind`, if they follow.
fn keywords(p: &mut Parser<'_>, words: &[K], kind: SyntaxKind) -> Option<CompletedMarker> {
    if !p.at_kws(words) {
        return None;
    }
    let m = p.start();
    p.eat_kws(words);
    Some(m.complete(p, kind))
}

#[cfg(test)]
mod tests;
