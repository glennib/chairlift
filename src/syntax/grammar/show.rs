//! `SHOW …` and `DESCRIBE`, as ClickHouse's `ParserTablePropertiesQuery`,
//! `ParserShowTablesQuery`, `ParserShowColumnsQuery`,
//! `ParserShowIndexesQuery`, `ParserShowSettingQuery`,
//! `ParserShowGrantsQuery`, `ParserShowCreateAccessEntityQuery` and
//! `ParserDescribeTableQuery` read them, each with the output clauses of
//! `ParserQueryWithOutput` after it.
//!
//! A `SHOW` is recognised by the keywords that name what it shows, listed
//! in [`HEADS`]; one that names something else is left verbatim. What the
//! common forms take after those keywords is parsed: `FROM` or `IN` and a
//! name, `[NOT] LIKE|ILIKE 'pattern'`, `WHERE`, `LIMIT`, the name of `SHOW
//! CREATE`, the users of `SHOW GRANTS FOR`. What a form takes beyond that,
//! such as the names of `SHOW CREATE ROW POLICY`, is kept as written,
//! [`Opaque`](SyntaxKind::Opaque) by design.
//!
//! # Keyword slots
//!
//! The keywords of [`HEADS`] are keywords right after `SHOW`; a word after
//! `SHOW CREATE` is the object's keyword only when a name follows it, so
//! `SHOW CREATE view` shows the table `view`. `DESCRIBE TABLE` likewise.

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::access;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::names;
use crate::syntax::grammar::select;
use crate::syntax::grammar::simple;
use crate::syntax::parser::Parser;

/// What follows a `SHOW` head.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    /// `FROM`, `IN`, `LIKE`, `WHERE`, `LIMIT`, in any order.
    Listing,
    /// A name, then what a listing takes.
    Named,
    /// `FOR` users, `WITH IMPLICIT`, `FINAL`.
    Grants,
}

/// The keywords after `SHOW` that this grammar knows, longest first where
/// one starts another, and what follows them.
const HEADS: &[(&[K], Form)] = &[
    (&[K::Full, K::Temporary, K::Tables], Form::Listing),
    (&[K::Full, K::Tables], Form::Listing),
    (&[K::Temporary, K::Tables], Form::Listing),
    (&[K::Tables], Form::Listing),
    (&[K::Databases], Form::Listing),
    (&[K::Dictionaries], Form::Listing),
    (&[K::Extended, K::Full, K::Columns], Form::Listing),
    (&[K::Extended, K::Full, K::Fields], Form::Listing),
    (&[K::Extended, K::Columns], Form::Listing),
    (&[K::Extended, K::Fields], Form::Listing),
    (&[K::Full, K::Columns], Form::Listing),
    (&[K::Full, K::Fields], Form::Listing),
    (&[K::Columns], Form::Listing),
    (&[K::Fields], Form::Listing),
    (&[K::Extended, K::Index], Form::Listing),
    (&[K::Extended, K::Indexes], Form::Listing),
    (&[K::Extended, K::Indices], Form::Listing),
    (&[K::Extended, K::Keys], Form::Listing),
    (&[K::Index], Form::Listing),
    (&[K::Indexes], Form::Listing),
    (&[K::Indices], Form::Listing),
    (&[K::Keys], Form::Listing),
    (&[K::Changed, K::Settings], Form::Listing),
    (&[K::Settings], Form::Listing),
    (&[K::Setting], Form::Named),
    (&[K::Processlist], Form::Listing),
    (&[K::Functions], Form::Listing),
    (&[K::Engines], Form::Listing),
    (&[K::Clusters], Form::Listing),
    (&[K::Cluster], Form::Named),
    (&[K::Grants], Form::Grants),
];

/// Whether the statement at the current token is one of this module's.
pub(crate) fn at_statement(p: &Parser<'_>) -> bool {
    match p.nth_keyword(0) {
        Some(K::Show) => p.nth_at_kw(1, K::Create) || head(p, 1).is_some(),
        Some(K::Describe | K::Desc) => !(p.nth_at_kw(1, K::Filesystem) && p.nth_at_kw(2, K::Cache)),
        _ => false,
    }
}

/// The [`HEADS`] entry at the `n`th token.
fn head(p: &Parser<'_>, n: usize) -> Option<(&'static [K], Form)> {
    HEADS.iter().copied().find(|(words, _)| {
        words
            .iter()
            .enumerate()
            .all(|(i, w)| p.nth_at_kw(n + i, *w))
    })
}

/// A statement of this module's, up to the end of the statement.
pub(crate) fn statement(p: &mut Parser<'_>) -> Option<()> {
    if p.at_kw(K::Show) {
        show(p)
    } else {
        describe(p)
    }
}

/// A [`ShowQuery`](SyntaxKind::ShowQuery).
fn show(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    if p.eat_kw(K::Create) {
        show_create(p)?;
    } else {
        let (words, form) = head(p, 0)?;
        for _ in words {
            p.bump_keyword();
        }
        parsed_or_opaque(p, |p| {
            match form {
                Form::Named => {
                    names::compound_name(p)?;
                }
                Form::Grants => {
                    if p.eat_kw(K::For) {
                        let allow = access::Allow {
                            all: true,
                            any: false,
                            current_user: true,
                        };
                        access::role_set(p, allow, &|p| {
                            p.at_kws(&[K::With, K::Implicit])
                                || p.at_kw(K::Final)
                                || simple::at_output_clause(p)
                        })?;
                    }
                    p.eat_kws(&[K::With, K::Implicit]);
                    p.eat_kw(K::Final);
                    return Some(());
                }
                Form::Listing => {}
            }
            listing(p)
        });
    }
    simple::output_clauses(p);
    m.complete(p, SyntaxKind::ShowQuery);
    Some(())
}

/// What follows `SHOW CREATE`: an access entity's keywords and what follows
/// them, or `[TEMPORARY] [TABLE | VIEW | DICTIONARY | DATABASE]` and a
/// name.
fn show_create(p: &mut Parser<'_>) -> Option<()> {
    if let Some((entity, words)) =
        access::entity(p, 0).filter(|&(_, words)| !p.nth_at(words, T::Dot))
    {
        for _ in 0..words {
            p.bump_keyword();
        }
        // A policy's name may spell `SETTINGS`: `ParserRowPolicyNames`
        // reads it before any output clause.
        if entity == access::Entity::RowPolicy && !p.at_end() {
            parsed_or_opaque(p, |p| access::row_policy_names(p).map(drop));
            return Some(());
        }
        if !(p.at_end() || simple::at_output_clause(p)) {
            fallback::opaque_until(p, &simple::at_output_clause)?;
        }
        return Some(());
    }
    if p.at_kw(K::Temporary) && p.nth_at_kw(1, K::Table) {
        p.bump_keyword();
    }
    if matches!(
        p.nth_keyword(0),
        Some(K::Table | K::View | K::Dictionary | K::Database)
    ) && names::at_name(p, 1)
    {
        p.bump_keyword();
    }
    names::compound_name(p).map(drop)
}

/// `FROM` or `IN` and a name, `[NOT] LIKE|ILIKE` and a string, `WHERE` and
/// an expression, `LIMIT` and an expression, as a listing takes them.
fn listing(p: &mut Parser<'_>) -> Option<()> {
    loop {
        if p.at_kw(K::From) || p.at_kw(K::In) {
            p.bump_keyword();
            names::compound_name(p)?;
        } else if matches!(p.nth_keyword(0), Some(K::Like | K::Ilike))
            || (p.at_kw(K::Not) && matches!(p.nth_keyword(1), Some(K::Like | K::Ilike)))
        {
            p.eat_kw(K::Not);
            p.bump_keyword();
            exprs::string(p)?;
        } else if p.at_kw(K::Where) || p.at_kw(K::Limit) {
            p.bump_keyword();
            exprs::expr(p)?;
        } else {
            return Some(());
        }
    }
}

/// `parse`, if it reaches the end of the statement or an output clause;
/// otherwise what it would have parsed, kept as written.
fn parsed_or_opaque(p: &mut Parser<'_>, parse: impl FnOnce(&mut Parser<'_>) -> Option<()>) {
    let checkpoint = p.checkpoint();
    if parse(p).is_some() && (p.at_end() || simple::at_output_clause(p)) {
        return;
    }
    p.rollback(checkpoint);
    if !(p.at_end() || simple::at_output_clause(p)) {
        fallback::opaque_until(p, &simple::at_output_clause);
    }
}

/// A [`DescribeQuery`](SyntaxKind::DescribeQuery).
fn describe(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    if p.at_kws(&[K::Temporary, K::Table]) {
        p.bump_keyword();
    }
    if p.at_kw(K::Table) && (names::at_name(p, 1) || p.nth_at(1, T::OpeningRoundBracket)) {
        p.bump_keyword();
    }
    if select::at_subquery(p, 0) {
        select::subquery(p)?;
    } else {
        let name = names::compound_name(p)?;
        if p.at(T::OpeningRoundBracket) {
            let call = name.precede(p);
            exprs::arg_list(p)?;
            call.complete(p, SyntaxKind::FunctionCall);
        }
    }
    simple::output_clauses(p);
    m.complete(p, SyntaxKind::DescribeQuery);
    Some(())
}

#[cfg(test)]
mod tests;
