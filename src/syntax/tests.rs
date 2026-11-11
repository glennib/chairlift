//! The tree as a whole: statements, trivia placement, the facts for the
//! verifier, re-casing and statistics.

use crate::lexer::Mode;
use crate::syntax::Entry;
use crate::syntax::FallbackLevel;
use crate::syntax::StatementOutcome;
use crate::syntax::SyntaxKind;
use crate::syntax::parse;
use crate::syntax::parse_as;
use crate::syntax::test_support::file_tree;
use crate::verify::verify;

#[test]
fn statements_hold_their_body_and_semicolon() {
    insta::assert_snapshot!(file_tree(
        "-- header\n\nDROP TABLE t; ;\nSELECT 1 -- trailing\n;\nSET x = 1",
        Mode::Sql,
        Entry::File
    ));
}

/// Trivia lies between siblings, in the deepest node that holds the tokens
/// on both sides: after a comma in the list, between operands, before the
/// `;` in the statement, and between statements in the root.
#[test]
fn trivia_lies_between_siblings() {
    insta::assert_snapshot!(file_tree(
        "SELECT -- after select\n    f( /* first */ a, -- after a\n    b /* before ) */ ),\n    \
         x {# jinja comment #} AND {% if c %} y {% endif %}\n; -- after ;\n",
        Mode::Jinja,
        Entry::SelectListProbe
    ));
    let parse = parse_as(
        "SELECT /* a */ f( /* b */ x /* c */ ) /* d */;",
        Mode::Sql,
        Entry::SelectListProbe,
    );
    for node in parse.syntax().descendants().skip(1) {
        let first = node.first_token().map(|t| t.kind());
        let last = node.last_token().map(|t| t.kind());
        assert!(first.is_some_and(|k| !k.is_trivia()), "{node:?}");
        assert!(last.is_some_and(|k| !k.is_trivia()), "{node:?}");
    }
}

#[test]
fn insert_data_is_one_token() {
    insta::assert_snapshot!(file_tree(
        "INSERT INTO t VALUES (1, 'a'),\n(2, 'b');\nINSERT INTO t FORMAT CSV\n1,2\n\nSELECT 1",
        Mode::Sql,
        Entry::File
    ));
}

#[test]
fn every_statement_is_verbatim_until_its_grammar_exists() {
    for source in ["SYSTEM FLUSH LOGS", "ALTER TABLE t ADD COLUMN a Int8"] {
        let parse = parse(source, Mode::Sql);
        let [statement] = &parse.statements()[..] else {
            panic!("{source}: one statement");
        };
        assert_eq!(statement.outcome, StatementOutcome::Verbatim, "{source}");
        assert!(parse.fallbacks().is_empty(), "{source}");
        assert_eq!(parse.recased(), source);
    }
}

#[test]
fn statement_kinds_come_from_the_leading_words() {
    let source = "select 1; with x as (select 1) select x; (select 1); \
                  create table t (a Int8); create or replace temporary table t (a Int8); \
                  create materialized view v as select 1; create view v as select 1; \
                  drop table if exists t; alter table t delete where 1; set x = 1; \
                  insert into t values (1); system flush logs; show create table t; ;";
    let kinds: Vec<String> = parse(source, Mode::Sql)
        .statements()
        .into_iter()
        .map(|s| s.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            "select",
            "with",
            "select",
            "create table",
            "create table",
            "create materialized view",
            "create view",
            "drop table",
            "alter table",
            "set",
            "insert",
            "system flush",
            "show create",
            "empty",
        ]
    );
}

#[test]
fn statistics_tell_opaque_regions_from_fallbacks() {
    let parse = parse_as(
        "SELECT sum(x) OVER (PARTITION BY y);\nSELECT sum(x) OVER (PARTITION BY y), f(a b)",
        Mode::Sql,
        Entry::SelectListProbe,
    );
    let stats = parse.statements();
    assert_eq!(stats[0].outcome, StatementOutcome::Opaque);
    assert_eq!(stats[0].opaque, 1);
    assert_eq!(stats[1].outcome, StatementOutcome::Partial);
    assert_eq!(stats[1].opaque, 1);
}

#[test]
fn statistics_count_items_and_fallbacks() {
    let parse = parse_as(
        "SELECT f(a, b c), (1, 2);\nSELECT 1;\nDROP TABLE t;",
        Mode::Sql,
        Entry::SelectListProbe,
    );
    let stats = parse.statements();
    assert_eq!(stats[0].outcome, StatementOutcome::Partial);
    // The two list items of `ExprList`, two of `f(…)` and two of the tuple.
    assert_eq!(stats[0].items, 6);
    assert_eq!(stats[0].verbatim_items, 1);
    assert_eq!(stats[0].range, 0..25);
    assert_eq!(stats[1].outcome, StatementOutcome::Parsed);
    assert_eq!(stats[2].outcome, StatementOutcome::Verbatim);
    let [fallback] = parse.fallbacks() else {
        panic!("one fallback");
    };
    assert_eq!(fallback.level, FallbackLevel::Item);
    assert_eq!(fallback.range, 12..15);
    assert_eq!(
        fallback.message,
        "expected `,` or the end of the list, found a word"
    );
}

#[test]
fn a_statement_that_does_not_parse_is_verbatim_with_its_reason() {
    let parse = parse_as("SELECT 1 +", Mode::Sql, Entry::Expression);
    assert_eq!(parse.statements()[0].outcome, StatementOutcome::Verbatim);
    let [fallback] = parse.fallbacks() else {
        panic!("one fallback");
    };
    assert_eq!(fallback.level, FallbackLevel::Statement);
    assert_eq!(fallback.range, 0..10);
}

/// The facts mark keyword slots for re-casing and literals read as text for
/// keeping, by token index; re-casing applies the first and keeps the second.
#[test]
fn facts_and_recasing() {
    let source = "select x::Int8, [1,null]::Array(Nullable(Int8)), cast('a' as String), null";
    let parse = parse_as(source, Mode::Sql, Entry::SelectListProbe);
    let recased = parse.recased();
    assert_eq!(
        recased,
        "SELECT x::Int8, [1,null]::Array(Nullable(Int8)), cast('a' AS String), NULL"
    );
    let facts = parse.facts();
    let tokens = crate::split::split(source, Mode::Sql);
    let text = |i: usize| tokens.tokens()[i].text(source);
    let regions: Vec<String> = facts
        .verbatim_regions()
        .iter()
        .map(|&(first, last)| (first..=last).map(text).collect())
        .collect();
    assert_eq!(regions, ["[1,null]", "'a'"]);
    assert!(facts.is_recased(0));
    verify(source, &recased, Mode::Sql, facts).expect("re-casing verifies");
    // Moving the whitespace inside a literal read as text is caught.
    let moved = recased.replace("[1,null]", "[1, null]");
    assert!(verify(source, &moved, Mode::Sql, facts).is_err());
}

#[test]
fn jinja_block_tags_between_statements_are_in_the_root() {
    let source = "{% if prod %}\nDROP TABLE t;\n{% endif %}\n{{ config(x=1) }}\nSELECT 1;\n";
    let parse = parse(source, Mode::Jinja);
    let root = parse.syntax();
    let kinds: Vec<SyntaxKind> = root
        .children_with_tokens()
        .map(|e| e.kind())
        .filter(|k| *k != SyntaxKind::Whitespace)
        .collect();
    assert_eq!(
        kinds,
        [
            SyntaxKind::JinjaBlock,
            SyntaxKind::Statement,
            SyntaxKind::JinjaBlock,
            SyntaxKind::Statement,
        ]
    );
    assert_eq!(root.text(), source);
}
