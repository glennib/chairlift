//! The expression probe: `SELECT <list>` statements parsed as an expression
//! list, to measure the expression grammar on real queries (see
//! [`Entry::SelectListProbe`](crate::syntax::Entry::SelectListProbe)).

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::exprs::AliasMode;
use crate::syntax::grammar::fallback;
use crate::syntax::parser::Parser;

/// A statement: `SELECT <list>` as an
/// [`ExpressionProbe`](SyntaxKind::ExpressionProbe), anything else verbatim.
pub(crate) fn statement(p: &mut Parser<'_>) {
    if !candidate(p) {
        fallback::verbatim_statement(p);
        return;
    }
    fallback::statement(p, |p| {
        let m = p.start();
        p.bump_keyword();
        common::expr_list(p, &fallback::never, AliasMode::Implicit)?;
        m.complete(p, SyntaxKind::ExpressionProbe);
        Some(())
    });
}

/// Whether the statement is `SELECT` and a list with no clause after it:
/// no `DISTINCT`, `ALL` or `TOP` before the list, and none of the words that
/// start a later clause outside brackets. A statement that uses such a word
/// as a name is left out too.
fn candidate(p: &Parser<'_>) -> bool {
    if !p.at_kw(K::Select)
        || p.nth(1).is_none()
        || matches!(p.nth_keyword(1), Some(K::Distinct | K::All | K::Top))
    {
        return false;
    }
    let mut depth = 0usize;
    let mut n = 1;
    while let Some(kind) = p.nth(n) {
        if fallback::is_opener(kind) {
            depth += 1;
        } else if fallback::is_closer(kind) {
            depth = depth.saturating_sub(1);
        } else if depth == 0
            && (kind == T::PipeOperator
                || matches!(
                    p.nth_keyword(n),
                    Some(
                        K::From
                            | K::Where
                            | K::Prewhere
                            | K::Group
                            | K::Having
                            | K::Order
                            | K::Limit
                            | K::Offset
                            | K::Settings
                            | K::Format
                            | K::Union
                            | K::Except
                            | K::Intersect
                            | K::Into
                            | K::Window
                            | K::Qualify
                            | K::With
                    )
                ))
        {
            return false;
        }
        n += 1;
    }
    true
}
