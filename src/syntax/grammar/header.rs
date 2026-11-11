//! A statement's header: Jinja expressions on lines of their own before it.

use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::parser::Parser;

/// The statement's [`JinjaHeader`](SyntaxKind::JinjaHeader), if it has one:
/// the `{{ … }}` tokens at its start that each end their line, with more of
/// the statement after them, such as dbt's `{{ config(…) }}`. Whatever they
/// render, it ends before the line the statement starts on, so they are no
/// part of its syntax. The first may follow the statement before on its
/// line (`…; {{ hint() }}`); one that shares its line with what follows, as
/// in `{{ verb }} INTO t …`, is part of the statement.
pub(crate) fn header(p: &mut Parser<'_>) {
    let mut n = 0;
    while p.nth_at(n, T::JinjaExpr)
        && (n == 0 || p.line_break_before(n))
        && p.line_break_before(n + 1)
    {
        n += 1;
    }
    if n == 0 {
        return;
    }
    let m = p.start();
    for _ in 0..n {
        p.bump();
    }
    m.complete(p, SyntaxKind::JinjaHeader);
}
