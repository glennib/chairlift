//! `INSERT`.
//!
//! The query of `INSERT … SELECT` is the `SELECT` grammar's, so the tree
//! dumps here print it as `<query>`.

use std::fmt::Write as _;

use rowan::NodeOrToken;

use crate::lexer::Mode;
use crate::syntax::Entry;
use crate::syntax::StatementOutcome;
use crate::syntax::SyntaxKind;
use crate::syntax::SyntaxNode;
use crate::syntax::parse;
use crate::syntax::test_support::recased;

/// `node` without whitespace, one element per line, with the query of
/// `INSERT … SELECT` as `<query>`.
fn render(node: &SyntaxNode, depth: usize, out: &mut String) {
    out.extend(std::iter::repeat_n("  ", depth));
    let query = matches!(
        node.kind(),
        SyntaxKind::SelectQuery | SyntaxKind::UnionQuery
    ) && node
        .parent()
        .is_some_and(|p| p.kind() == SyntaxKind::InsertQuery);
    if query {
        out.push_str("<query>\n");
        return;
    }
    out.push_str(node.kind().name());
    out.push('\n');
    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Node(n) => render(&n, depth + 1, out),
            NodeOrToken::Token(t) if t.kind() != SyntaxKind::Whitespace => {
                out.extend(std::iter::repeat_n("  ", depth + 1));
                let _ = writeln!(out, "{} {:?}", t.kind().name(), t.text());
            }
            NodeOrToken::Token(_) => {}
        }
    }
}

/// Each input and the bodies of its statements.
fn trees(inputs: &[&str]) -> String {
    let mut out = String::new();
    for input in inputs {
        let _ = writeln!(out, "> {input}");
        let parse = parse(input, Mode::Jinja);
        assert_eq!(parse.syntax().text().to_string(), *input, "lossless");
        for statement in parse
            .syntax()
            .children()
            .filter(|n| n.kind() == SyntaxKind::Statement)
        {
            if let Some(body) = statement.first_child() {
                render(&body, 0, &mut out);
            }
        }
        out.push('\n');
    }
    out
}

/// Asserts that every statement of `source` parsed whole.
#[track_caller]
fn assert_parses(source: &str) {
    let parse = parse(source, Mode::Jinja);
    assert_eq!(parse.fallbacks(), &[], "{source}");
    for statement in parse.statements() {
        assert_eq!(statement.outcome, StatementOutcome::Parsed, "{source}");
    }
}

/// The outcome of the first statement of `source`.
fn outcome(source: &str) -> StatementOutcome {
    parse(source, Mode::Jinja).statements()[0].outcome
}

#[test]
fn targets_and_columns() {
    insta::assert_snapshot!(trees(&[
        "INSERT INTO t VALUES (1)",
        "insert into table db.t (a, b.c, `d`,) values (1, 2, 3)",
        "INSERT INTO t (* EXCEPT (b), t.* EXCEPT c, COLUMNS('a') APPLY toString, t.COLUMNS(x, y)) VALUES",
        "INSERT INTO FUNCTION file('f.csv', 'CSV', 'a UInt8') PARTITION BY a % 2 SELECT 1",
        "INSERT INTO TABLE FUNCTION remote('h', db.t) (a) VALUES (1)",
        "INSERT INTO {{ schema }}.events_{{ suffix }} (a) VALUES (1)",
        "INSERT INTO {db:Identifier}.{t:Identifier} VALUES (1)",
    ]));
}

#[test]
fn data_and_its_clauses() {
    insta::assert_snapshot!(trees(&[
        "INSERT INTO t VALUES (1, 'a'),\n(2, 'b');",
        "INSERT INTO t VALUES;",
        "INSERT INTO t FORMAT CSV\n1,2\n\nSELECT 1",
        "INSERT INTO t FORMAT JSONEachRow",
        "INSERT INTO t FORMAT Values (1)",
        "INSERT INTO t SETTINGS async_insert = 1, wait_for_async_insert = 0 VALUES (1)",
        "INSERT INTO t SETTINGS input_format_skip_unknown_fields FORMAT JSONEachRow {\"a\": 1}",
        "INSERT INTO t FROM INFILE 'data.csv.gz' COMPRESSION 'gzip' SETTINGS max_threads = 1 FORMAT CSV",
        "INSERT INTO t FROM INFILE 'x'",
    ]));
}

#[test]
fn queries() {
    insta::assert_snapshot!(trees(&[
        "INSERT INTO t (a, b) SELECT a, b FROM u",
        "INSERT INTO t SELECT 1 UNION ALL SELECT 2",
        "INSERT INTO t (SELECT 1)",
        "INSERT INTO t (a) (SELECT 1)",
        "INSERT INTO t FROM u",
        "INSERT INTO t WITH x AS (SELECT 1) SELECT * FROM x",
        "WITH x AS (SELECT 1), 2 AS y INSERT INTO t SELECT * FROM x",
        "INSERT INTO t SELECT * FROM input('a UInt8') FORMAT CSV\n1\n",
        "INSERT INTO t SELECT * FROM u SETTINGS max_threads = 1",
        "INSERT INTO t SELECT * FROM u FORMAT Null",
    ]));
}

/// The query, its `SETTINGS` and the `FORMAT` after it are parsed whole,
/// with the query as the `SELECT` grammar reads it.
#[test]
fn queries_parse_whole() {
    for source in [
        "INSERT INTO t (a, b) SELECT a, b FROM u WHERE x SETTINGS y = 1",
        "INSERT INTO t SELECT * FROM input('a UInt8') FORMAT CSV\n1\n",
        "WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x",
        "INSERT INTO t (a) SELECT 1 UNION ALL (SELECT 2)",
        "INSERT INTO FUNCTION null('a UInt8') SELECT 1",
    ] {
        assert_parses(source);
    }
}

/// The words ClickHouse reads as keywords at a place are keywords there,
/// and names elsewhere, whatever they spell.
#[test]
fn keyword_slots() {
    for (source, expected) in [
        (
            "insert into table table values (1)",
            "INSERT INTO TABLE table VALUES (1)",
        ),
        (
            "insert into values values (1)",
            "INSERT INTO values VALUES (1)",
        ),
        (
            "insert into format (format, values, select) format CSV",
            "INSERT INTO format (format, values, select) FORMAT CSV",
        ),
        (
            "insert into t (select) values (1)",
            "INSERT INTO t (select) VALUES (1)",
        ),
        (
            "insert into t (true.*, null.COLUMNS(a)) values (1)",
            "INSERT INTO t (true.*, null.COLUMNS(a)) VALUES (1)",
        ),
        (
            "insert into t settings format = 1 format TSV",
            "INSERT INTO t SETTINGS format = 1 FORMAT TSV",
        ),
        (
            "insert into t from infile 'x' compression 'gzip' format csv",
            "INSERT INTO t FROM INFILE 'x' COMPRESSION 'gzip' FORMAT csv",
        ),
        (
            "insert into function file('x') partition by a select 1",
            "INSERT INTO FUNCTION file('x') PARTITION BY a SELECT 1",
        ),
        (
            "with x as (select 1) insert into t select * from x",
            "WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x",
        ),
    ] {
        assert_eq!(recased(source, Entry::File), expected, "{source}");
    }
}

/// The data keeps its bytes, and no word in it is a keyword slot.
#[test]
fn the_data_is_one_token() {
    let source = "insert into t values (1, null, 'x'),\n  (2, true, 'y') ;";
    assert_eq!(
        recased(source, Entry::File),
        "INSERT INTO t VALUES (1, null, 'x'),\n  (2, true, 'y') ;"
    );
    let source = "insert into t format TSV\nselect\tfrom\n";
    assert_eq!(
        recased(source, Entry::File),
        "INSERT INTO t FORMAT TSV\nselect\tfrom\n"
    );
}

/// What ClickHouse does not read as an `INSERT` stays verbatim: a statement
/// with pipe syntax, settings after `FORMAT`, `VALUES` after `FROM INFILE`.
#[test]
fn what_does_not_parse_stays_verbatim() {
    for source in [
        "INSERT INTO t FROM u |> WHERE x",
        "INSERT INTO t SELECT 1 FORMAT JSON SETTINGS a = 1",
        "INSERT INTO t FROM INFILE 'x' VALUES (1)",
        "INSERT INTO t",
        "INSERT INTO table VALUES (1)",
    ] {
        assert_eq!(outcome(source), StatementOutcome::Verbatim, "{source}");
    }
    // A column that is no name, `*` or matcher is kept on its own.
    for source in [
        "INSERT INTO t (a, f(b)) VALUES (1)",
        "INSERT INTO t (a + 1) VALUES (1)",
    ] {
        assert_eq!(outcome(source), StatementOutcome::Partial, "{source}");
    }
}

/// Comments and Jinja block tags between the parts stay in the tree.
#[test]
fn comments_and_tags_between_clauses() {
    insta::assert_snapshot!(trees(&[
        "INSERT INTO t -- the target\n(a, b) /* c */ SELECT a, b FROM u",
        "INSERT INTO t {% if x %} SETTINGS a = 1 {% endif %} SELECT 1",
    ]));
}
