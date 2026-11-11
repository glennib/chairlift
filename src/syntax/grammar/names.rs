//! Names: identifiers, dotted names, and the Jinja expressions that stand
//! for them.

use crate::lexer::TokenKind;
use crate::syntax::SyntaxKind;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// Whether the `n`th token can be a name: a bare word (whatever it spells,
/// since ClickHouse reserves none), a quoted identifier, a Jinja expression
/// or raw block, or a `{name:Type}` parameter.
pub(crate) fn at_name(p: &Parser<'_>, n: usize) -> bool {
    match p.nth(n) {
        Some(
            TokenKind::BareWord
            | TokenKind::QuotedIdentifier
            | TokenKind::JinjaExpr
            | TokenKind::Verbatim,
        ) => true,
        Some(TokenKind::OpeningCurlyBrace) => at_query_parameter(p, n),
        _ => false,
    }
}

/// Whether the `n`th token is the identifier `from`, bare or quoted, which
/// ClickHouse gives no alias without `AS` (`ParserWithOptionalAlias`), so
/// that `SELECT x, from t` is not `from AS t`.
pub(crate) fn at_from(p: &Parser<'_>, n: usize) -> bool {
    match p.nth(n) {
        Some(TokenKind::BareWord) => p.nth_text(n).eq_ignore_ascii_case("from"),
        Some(TokenKind::QuotedIdentifier) => {
            let text = p.nth_text(n);
            text.len() == 6
                && text
                    .get(1..5)
                    .is_some_and(|w| w.eq_ignore_ascii_case("from"))
        }
        _ => false,
    }
}

/// How many tokens the name at the `n`th token spans, if one is there: one,
/// or the five of a `{name:Identifier}` parameter.
pub(crate) fn name_tokens(p: &Parser<'_>, n: usize) -> Option<usize> {
    if !at_query_parameter(p, n) {
        return at_name(p, n).then_some(1);
    }
    (p.nth_at(n + 3, TokenKind::BareWord) && p.nth_at(n + 4, TokenKind::ClosingCurlyBrace))
        .then_some(5)
}

/// Whether the `n`th token starts `{name:Type}`.
pub(crate) fn at_query_parameter(p: &Parser<'_>, n: usize) -> bool {
    p.nth_at(n, TokenKind::OpeningCurlyBrace)
        && p.nth_at(n + 1, TokenKind::BareWord)
        && p.nth_at(n + 2, TokenKind::Colon)
}

/// One name: a [`Name`](SyntaxKind::Name), or a
/// [`QueryParameter`](SyntaxKind::QueryParameter) (`{tbl:Identifier}`). A
/// bare word is consumed as a name whatever it spells.
pub(crate) fn name(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if at_query_parameter(p, 0) {
        return super::exprs::query_parameter(p);
    }
    if !at_name(p, 0) {
        return p.expected("a name");
    }
    let m = p.start();
    p.bump();
    Some(m.complete(p, SyntaxKind::Name))
}

/// A dotted name, `a`, `db.table` or `a.b.c`: one [`name`], or a
/// [`QualifiedName`](SyntaxKind::QualifiedName) of several. Also takes the
/// JSON path separators `.:`, `.^` and `.@`.
pub(crate) fn compound_name(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    compound_name_with(p, false)
}

/// [`compound_name`], stopping before a `.COLUMNS(…)` matcher when
/// `before_columns` is set, so that an expression can parse `t.COLUMNS(…)`
/// as one, as ClickHouse does for exactly that spelling.
pub(crate) fn compound_name_with(
    p: &mut Parser<'_>,
    before_columns: bool,
) -> Option<CompletedMarker> {
    let first = name(p)?;
    let mut qualified: Option<crate::syntax::parser::Marker> = None;
    loop {
        if !p.at(TokenKind::Dot) {
            break;
        }
        let separator = if matches!(
            p.nth(1),
            Some(TokenKind::Colon | TokenKind::Caret | TokenKind::At)
        ) {
            2
        } else {
            1
        };
        if !at_name(p, separator) {
            break;
        }
        if before_columns
            && separator == 1
            && p.nth_text(1) == "COLUMNS"
            && super::exprs::at_columns_matcher(p, 1)
        {
            break;
        }
        if qualified.is_none() {
            qualified = Some(first.precede(p));
        }
        for _ in 0..separator {
            p.bump();
        }
        name(p)?;
    }
    Some(match qualified {
        Some(m) => m.complete(p, SyntaxKind::QualifiedName),
        None => first,
    })
}
