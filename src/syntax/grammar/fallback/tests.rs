//! The fallback ladder on a small grammar of clauses.

use crate::keyword::Keyword as K;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::exprs::AliasMode;
use crate::syntax::grammar::fallback;
use crate::syntax::parser::Parser;
use crate::syntax::test_support::grammar;

fn at_clause(p: &Parser<'_>) -> bool {
    p.at_kw(K::Where) || p.at_kw(K::Having) || (p.at_kw(K::Group) && p.nth_at_kw(1, K::By))
}

/// Clauses in any order, as a statement grammar parses them: `WHERE e`,
/// `HAVING e`, `GROUP BY list`.
fn clauses(p: &mut Parser<'_>) -> Option<()> {
    while !p.at_end() {
        if !at_clause(p) {
            return p.expected("a clause");
        }
        fallback::clause(p, &at_clause, |p| {
            let m = p.start();
            let kind = if p.at_kw(K::Group) {
                p.bump_keyword();
                p.bump_keyword();
                common::expr_list(p, &at_clause, AliasMode::None)?;
                SyntaxKind::GroupByClause
            } else {
                let kind = if p.at_kw(K::Where) {
                    SyntaxKind::WhereClause
                } else {
                    SyntaxKind::HavingClause
                };
                p.bump_keyword();
                exprs::expr(p)?;
                kind
            };
            m.complete(p, kind);
            Some(())
        });
    }
    Some(())
}

#[test]
fn clauses_fall_back_one_at_a_time() {
    insta::assert_snapshot!(grammar(
        "WHERE a\n{% if x %}\nWHERE b\n{% else %}\nWHERE c d\n{% endif %}\nGROUP BY e, f g, h\nHAVING i",
        clauses
    ));
}

#[test]
fn a_verbatim_clause_ends_at_the_next_clause_keyword_outside_brackets() {
    insta::assert_snapshot!(grammar("WHERE a b (WHERE c) HAVING d", clauses));
}

#[test]
fn a_list_item_ends_at_the_list_end_of_its_clause() {
    // The item `f g` is kept verbatim up to the next clause, not beyond it.
    let tree = grammar("GROUP BY f g HAVING x", clauses);
    assert!(tree.contains("VerbatimItem"), "{tree}");
    assert!(tree.contains("HavingClause"), "{tree}");
}

#[test]
fn speculation_stays_linear() {
    // Every `INTERVAL` and `CASE` is tried as one and then read as a name,
    // nested: without the budget this would take exponential time.
    for word in ["INTERVAL", "CASE"] {
        let deep = 150;
        let source = format!(
            "{}x{}",
            format!("{word} (").repeat(deep),
            ") y".repeat(deep)
        );
        let started = std::time::Instant::now();
        let parse = crate::syntax::parse_as(
            &source,
            crate::lexer::Mode::Sql,
            crate::syntax::Entry::Expression,
        );
        assert_eq!(parse.syntax().text().to_string(), source);
        assert!(
            started.elapsed().as_secs() < 5,
            "{word}: {:?}",
            started.elapsed()
        );
    }
}
