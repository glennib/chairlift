//! The fallback ladder on a small grammar of clauses.

use crate::keyword::Keyword as K;
use crate::lexer::Mode;
use crate::syntax::Entry;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::exprs::AliasMode;
use crate::syntax::grammar::fallback;
use crate::syntax::parser::FUEL_PER_TOKEN;
use crate::syntax::parser::Parser;
use crate::syntax::test_support::grammar;

const FUEL_BASE: u64 = crate::syntax::parser::FUEL_BASE as u64;

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
                common::expr_list(p, &at_clause, AliasMode::As)?;
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

/// A text that repeats `open` and `close` around `middle`, `n` times each,
/// after `head`, parsed as `entry`.
struct Shape {
    entry: Entry,
    head: &'static str,
    open: &'static str,
    middle: &'static str,
    close: &'static str,
    tail: &'static str,
    /// How many times `open` and `close` repeat, at the smaller size; the
    /// larger is twice that. Nested shapes stay within the depth limit,
    /// so that it does not cut them short.
    n: usize,
    /// Steps per token the shape may take: what the grammar needs, with
    /// room to spare, and far below [`FUEL_PER_TOKEN`].
    per_token: u64,
}

const fn shape(open: &'static str, middle: &'static str, close: &'static str) -> Shape {
    Shape {
        entry: Entry::Expression,
        head: "",
        open,
        middle,
        close,
        tail: "",
        n: 30,
        per_token: 24,
    }
}

/// A shape of whole statements: `head`, `open` repeated, `middle`,
/// `close` repeated, `tail`. Clauses look ahead for the clause keywords
/// before and after each, so statements take more steps per token than
/// expressions: a chain of `JOIN t` takes 30.
const fn statement(
    head: &'static str,
    open: &'static str,
    middle: &'static str,
    close: &'static str,
    tail: &'static str,
) -> Shape {
    Shape {
        entry: Entry::File,
        head,
        tail,
        per_token: 40,
        ..shape(open, middle, close)
    }
}

/// A statement of `head`, then `item` repeated, then `tail`.
const fn run(head: &'static str, item: &'static str, tail: &'static str) -> Shape {
    Shape {
        n: 500,
        ..statement(head, item, "", "", tail)
    }
}

/// Shapes that tempt a grammar into re-reading what it has read: brackets
/// that are never closed, failures inside failures, alternatives tried
/// inside alternatives, and long lists of the same.
const SHAPES: &[Shape] = &[
    shape("(", "x", ""),
    shape("[", "x", ""),
    shape("f(", "x", ""),
    shape("x IN (", "x", ""),
    shape("overlay(", "x", ""),
    shape("columns(", "x", ""),
    shape("ARRAY[(a, ", "x", ""),
    shape("INTERVAL (", "x", ""),
    shape("CASE (", "x", ""),
    shape("f(a b, ", "x", ""),
    shape("{x:Array(", "x", ""),
    Shape {
        entry: Entry::DataType,
        ..shape("Tuple(a ", "x", "")
    },
    shape("INTERVAL (", "x", ") y"),
    shape("CASE (", "x", ") y"),
    shape("x = some(", "1", ", 1)"),
    shape("f(a b, ", "x", ")"),
    shape("overlay(", "x", " PLACING y FROM 1)"),
    shape("columns(", "x", ")"),
    shape("ARRAY[(a, ", "x", ")]"),
    shape("arrayMap((x) -> ", "x", ", a)"),
    shape("f(* APPLY (", "x", "))"),
    Shape {
        head: "x::",
        ..shape("Tuple(a ", "UInt8", ")")
    },
    // Alternatives that fail inside alternatives that fail: the speculation
    // budget, half the fuel, goes on them before the parse gives up trying.
    Shape {
        per_token: 56,
        ..shape("INTERVAL (", "x", ") + y")
    },
    Shape {
        per_token: 56,
        ..shape("CASE (", "x", ") + y")
    },
    Shape {
        per_token: 56,
        ..shape("x = some(", "a b", ") c")
    },
    // Queries.
    statement("", "SELECT * FROM (", "SELECT 1", ")", ""),
    statement("", "SELECT * FROM (", "SELECT 1", "", ""),
    statement("SELECT * FROM ", "(", "t", ")", ""),
    statement("SELECT * FROM ", "(", "a JOIN b ON 1", ")", ""),
    statement("SELECT * FROM ", "(", "t", "", ""),
    statement("SELECT 1 ORDER BY ", "(", "x", ")", ""),
    statement("SELECT 1 ORDER BY ", "(", "x", "", ""),
    statement(
        "SELECT 1 WHERE x IN ",
        "(SELECT y FROM t WHERE y IN ",
        "(1)",
        ")",
        "",
    ),
    statement("", "WITH x AS (", "SELECT 1", ") SELECT 1", ""),
    run("WITH ", "x AS (SELECT 1), ", "y AS (SELECT 2) SELECT 1"),
    run("", "SELECT 1 UNION ALL ", "SELECT 1"),
    run("", "(SELECT 1) UNION DISTINCT ", "(SELECT 1)"),
    run("SELECT * FROM t", " JOIN t ON 1", ""),
    run("SELECT * FROM t", " LEFT ARRAY JOIN a", ""),
    run("SELECT * FROM t", " JOIN t", ""),
    run("SELECT ", "a, ", "b FROM t"),
    run("SELECT 1 FROM t WHERE ", "a AND b OR ", "c"),
    run("SELECT 1 FROM t ", "WHERE a ", ""),
    // Tables.
    statement(
        "CREATE TABLE t (a ",
        "Array(",
        "UInt8",
        ")",
        ") ENGINE = Memory",
    ),
    statement(
        "CREATE TABLE t (a ",
        "Tuple(b ",
        "UInt8",
        ")",
        ") ENGINE = Memory",
    ),
    statement("CREATE TABLE t (a ", "Array(", "UInt8", "", ""),
    statement("CREATE TABLE t (a Int8 DEFAULT ", "(", "1", ")", ")"),
    statement("CREATE TABLE t (a Int8 DEFAULT ", "(", "1", "", ""),
    statement(
        "CREATE TABLE t (a Int8) ENGINE = MergeTree ORDER BY ",
        "(",
        "a",
        ")",
        "",
    ),
    statement(
        "CREATE MATERIALIZED VIEW v AS ",
        "SELECT * FROM (",
        "SELECT 1",
        ")",
        "",
    ),
    run(
        "CREATE TABLE t (",
        "c Int8 DEFAULT 1 CODEC(ZSTD(1)), ",
        "d Int8) ENGINE = Memory",
    ),
    run(
        "CREATE TABLE t (",
        "c Int8 BAD, ",
        "d Int8) ENGINE = Memory",
    ),
    run(
        "CREATE TABLE t (a Int8) ENGINE = MergeTree ORDER BY a TTL ",
        "a + 1, ",
        "a",
    ),
    // Brackets that are never closed, at a length a re-scan per bracket
    // would show.
    unclosed("", "(SELECT "),
    unclosed("", "(SELECT 1 UNION ALL "),
    unclosed("", "(("),
    unclosed("", "WITH x AS ("),
    unclosed("", "WITH x AS (SELECT * FROM x WHERE x IN ("),
    unclosed("WITH x (", "a, "),
    unclosed("SELECT * FROM ", "(a JOIN "),
    unclosed("SELECT * FROM ", "((a, b), "),
    unclosed("SELECT * FROM ", "(SELECT * FROM "),
    unclosed("SELECT * FROM ", "view(SELECT * FROM "),
    unclosed("SELECT * FROM ", "f("),
    unclosed("SELECT * FROM t JOIN u ON ", "("),
    unclosed(
        "SELECT * FROM t WHERE x IN ",
        "(SELECT x FROM t WHERE x IN ",
    ),
    unclosed("SELECT ", "(from "),
    unclosed("SELECT ", "(with "),
    unclosed("CREATE TABLE t (", "a UInt8, "),
    unclosed("CREATE TABLE t (a UInt8 CODEC(", "ZSTD("),
    unclosed("CREATE TABLE t (a UInt8, ", "PROJECTION p (SELECT "),
    unclosed("CREATE TABLE t (a UInt8, INDEX i ", "("),
    unclosed("CREATE TABLE t (a UInt8) ENGINE = ", "f("),
    unclosed(
        "CREATE TABLE t (a UInt8) ENGINE = MergeTree ORDER BY ",
        "tuple(",
    ),
    unclosed("CREATE TABLE t AS ", "SELECT * FROM ("),
    // Nesting that closes, where each level looks into the brackets it
    // opens: for a query after `FROM` or `WITH`, a table, a join.
    statement("SELECT ", "(from ", "(x, y)", ")", ""),
    statement("SELECT ", "(with x AS ", "(SELECT 1)", " SELECT 1)", ""),
    statement("SELECT * FROM ", "(a JOIN ", "b ON 1", ") ON 1", ""),
    statement(
        "SELECT * FROM ",
        "viewIfPermitted(SELECT * FROM ",
        "t",
        " ELSE f())",
        "",
    ),
    statement(
        "CREATE TABLE t AS ",
        "with x AS (",
        "SELECT 1",
        ") SELECT 1",
        "",
    ),
    // Long runs of what a clause or list reads, well-formed or not.
    run("SELECT * FROM a", ", a b c", ""),
    run("SELECT * FROM t", " GLOBAL ANY LEFT OUTER", ""),
    run("SELECT * FROM t JOIN u ON 1", ", v", ""),
    run("SELECT * FROM t JOIN u USING a", ", a", ""),
    // Each join's condition is the next join's keyword, so every join falls
    // back and its keywords are read again as the next clause's.
    Shape {
        per_token: 48,
        ..run("SELECT * FROM t", " JOIN t ON ", "1")
    },
    run("", "SELECT 1 UNION ", ""),
    run("SELECT 1", " WHERE 1 1", ""),
    run("SELECT ", "a b c, ", "1"),
    run("SELECT 1 SETTINGS a = 1", ", a", ""),
    run("SELECT 1", " LIMIT 1 BY a", ""),
    run("SELECT 1 ORDER BY a", " WITH FILL", ""),
    run("SELECT 1 FORMAT JSON", " SETTINGS a = 1", ""),
    run("SELECT * FROM f(", "SETTINGS a = 1, ", ")"),
    run("FROM t", " select", ""),
    run("", "SELECT 1; ", ""),
    run("CREATE TABLE t AS with ", "a, ", "b"),
    run(
        "CREATE TABLE t (d Date) ENGINE = MergeTree ORDER BY d TTL d GROUP BY a SET a = 1",
        ", a = 1",
        "",
    ),
    run(
        "CREATE TABLE t (d Date) ENGINE = MergeTree ORDER BY d TTL d GROUP BY ",
        "a, ",
        "b",
    ),
    run("CREATE TABLE t (a UInt8)", " ENGINE = Memory", ""),
    run("CREATE TABLE t (a UInt8)", " ENGINE = Memory 1", ""),
    run(
        "CREATE TABLE t (a UInt8 CODEC(",
        "ZSTD(1), ",
        "NONE)) ENGINE = Memory",
    ),
    run(
        "CREATE TABLE t (a UInt8) ENGINE = MergeTree ORDER BY (",
        "a DESC, ",
        "b)",
    ),
    run("CREATE MATERIALIZED VIEW v REFRESH EVERY ", "1 HOUR ", ""),
    run(
        "CREATE MATERIALIZED VIEW v REFRESH EVERY 1 HOUR DEPENDS ON a",
        ", a",
        "",
    ),
    run(
        "CREATE TABLE t (a UInt8) ENGINE = Kafka SETTINGS ",
        "a = 1, ",
        "b",
    ),
    // INSERT, and the statements of a line or two.
    run("INSERT INTO t (", "a, ", "b) VALUES (1)"),
    run("INSERT INTO t (", "a b, ", "b) SELECT 1"),
    run("INSERT INTO t (", "t.* EXCEPT (a), ", "b) SELECT 1"),
    run(
        "WITH ",
        "x AS (SELECT 1), ",
        "y AS (SELECT 2) INSERT INTO t SELECT 1",
    ),
    run("INSERT INTO t SETTINGS ", "a = 1, ", "b SELECT 1"),
    run("INSERT INTO t SELECT 1", " FORMAT CSV", ""),
    statement("INSERT INTO t ", "(", "SELECT 1", ")", ""),
    statement("INSERT INTO t (a) ", "(", "SELECT 1", ")", ""),
    unclosed("INSERT INTO t ", "("),
    unclosed("INSERT INTO t (a) ", "(SELECT "),
    unclosed("WITH x AS (SELECT 1) INSERT INTO t ", "("),
    run("DROP TABLE ", "a, ", "b"),
    run("DROP TABLE ", "a b, ", "b"),
    run("DROP TABLE t", " FORMAT Null", ""),
    run("RENAME TABLE ", "a TO b, ", "c TO d"),
    run("EXCHANGE TABLES ", "a AND b, ", "c AND d"),
    run("SET ", "a = 1, ", "b = 2"),
    run("OPTIMIZE TABLE t DEDUPLICATE BY ", "a, ", "b"),
    run("OPTIMIZE TABLE t DRY RUN PARTS ", "'a', ", "'b'"),
    statement("OPTIMIZE TABLE t PARTITION ", "(", "1", ")", " FINAL"),
    unclosed("OPTIMIZE TABLE t PARTITION ", "("),
    run("ATTACH TABLE t ", "(a UInt8) ", ""),
];

/// A statement of `head` and `open` repeated, never closed, at lengths
/// past the depth limit.
const fn unclosed(head: &'static str, open: &'static str) -> Shape {
    Shape {
        n: 1000,
        ..statement(head, open, "", "", "")
    }
}

/// Lists of items like those above, as the probe's `SELECT` list.
const LISTS: &[&str] = &[
    "case (x) y, ",
    "interval (x) y, ",
    "x = some(1, 2), ",
    "f(a b), ",
    "position(+), x IN (1), ",
    "CASE WHEN a THEN CASE WHEN b THEN c END END, ",
];

/// The steps and significant tokens of `source` parsed as `entry`, with a
/// check that it did not run out of fuel.
fn steps(source: &str, entry: Entry) -> (u64, u64) {
    let parse = crate::syntax::parse_as(source, Mode::Sql, entry);
    assert_eq!(parse.syntax().text().to_string(), source, "lossless");
    assert!(
        !parse
            .fallbacks()
            .iter()
            .any(|f| f.message.contains("too much work")),
        "{source}: out of fuel"
    );
    let tokens = crate::split::split(source, Mode::Sql)
        .tokens()
        .iter()
        .filter(|t| t.kind().is_significant())
        .count();
    (parse.steps(), tokens as u64)
}

/// The parse's steps stay within a small multiple of the tokens for every
/// shape, and grow no faster than they do: at twice the size, about twice
/// the steps. Nested shapes stay within the depth limit, so that the depth
/// limit does not cut them short.
#[test]
fn steps_stay_linear() {
    let mut texts = Vec::new();
    for shape in SHAPES {
        let text = |n: usize| {
            format!(
                "{}{}{}{}{}",
                shape.head,
                shape.open.repeat(n),
                shape.middle,
                shape.close.repeat(n),
                shape.tail
            )
        };
        texts.push((
            text(shape.n),
            text(2 * shape.n),
            shape.entry,
            shape.per_token,
        ));
    }
    for item in LISTS {
        let text = |n: usize| format!("SELECT {}x", item.repeat(n));
        texts.push((text(500), text(1000), Entry::SelectListProbe, 24));
    }
    let mut failures = Vec::new();
    for (small, large, entry, per_token) in texts {
        let (small_steps, small_tokens) = steps(&small, entry);
        let (large_steps, large_tokens) = steps(&large, entry);
        let shape: String = small.chars().take(80).collect();
        for (steps, tokens) in [(small_steps, small_tokens), (large_steps, large_tokens)] {
            if steps > per_token * tokens + FUEL_BASE {
                failures.push(format!("{shape}…: {steps} steps for {tokens} tokens"));
            }
        }
        if large_steps * 10 > small_steps * 25 + 10 * FUEL_BASE {
            failures.push(format!(
                "{shape}…: {small_steps} steps, then {large_steps} at twice the size"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A statement that would take more than its fuel is kept verbatim, having
/// taken exactly its fuel.
#[test]
fn a_statement_out_of_fuel_is_verbatim() {
    let source = "a b c d";
    let split = crate::split::split(source, Mode::Sql);
    let parse = crate::syntax::parse_with(source, &split, &mut |p| {
        fallback::statement(p, |p| {
            while p.nth(0).is_some() {}
            Some(())
        });
    });
    assert_eq!(
        parse.fallbacks()[0].message,
        "the statement takes too much work to parse"
    );
    let body = parse
        .syntax()
        .children()
        .find(|n| n.kind() == SyntaxKind::Statement)
        .and_then(|s| s.first_child())
        .map(|n| n.kind());
    assert_eq!(body, Some(SyntaxKind::VerbatimStatement));
    assert_eq!(parse.steps(), 4 * FUEL_PER_TOKEN as u64 + FUEL_BASE);
}
