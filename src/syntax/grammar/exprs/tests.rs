//! Expressions: tree snapshots per construct, precedence against
//! ClickHouse, and the keyword slots.

use std::fmt::Write as _;

use crate::syntax::Entry;
use crate::syntax::test_support::expr;
use crate::syntax::test_support::parenthesized;
use crate::syntax::test_support::recased;

/// The trees of `inputs`, each under its input, for one snapshot.
fn trees(inputs: &[&str]) -> String {
    let mut out = String::new();
    for input in inputs {
        out.push_str("> ");
        out.push_str(input);
        out.push('\n');
        out.push_str(&expr(input));
        out.push('\n');
    }
    out
}

#[test]
fn literals() {
    insta::assert_snapshot!(trees(&[
        "1",
        "0x1F",
        "'it''s'",
        "$$text$$",
        "NULL",
        "null",
        "true",
        "FALSE",
        "inf",
        "NaN",
        "-1",
        "- 1.5",
        "-inf",
        "+1",
        "DATE '2026-01-01'",
        "TIMESTAMP '2026-01-01 00:00:00'",
        "date",
    ]));
}

#[test]
fn names() {
    insta::assert_snapshot!(trees(&[
        "a",
        "db.t.c",
        "`quoted name`",
        "\"double\"",
        "{{ column }}",
        "events_{{ suffix }}",
        "{{ schema }}.events",
        "{tbl:Identifier}",
        "json.a.:Int64",
        "key",
        "t.from",
    ]));
}

#[test]
fn operators() {
    insta::assert_snapshot!(trees(&[
        "a NOT LIKE 'x%' ESCAPE '!'",
        "a ILIKE b",
        "a SIMILAR TO 'x'",
        "a NOT SIMILAR TO 'x'",
        "a REGEXP 'x'",
        "a !~* 'x'",
        "a GLOBAL NOT IN (1, 2)",
        "a IS NOT DISTINCT FROM b",
        "a IS TRUE",
        "x AT TIME ZONE 'UTC'",
        "x AT LOCAL",
        "a <=> b",
        "a MOD b DIV c",
        "a || b",
        "f | g",
        "a NOT BETWEEN 1 AND 2",
        "c ? a : b",
    ]));
}

#[test]
fn and_or_runs_are_one_node() {
    insta::assert_snapshot!(trees(&[
        "a AND b AND c",
        "a OR b AND c OR d",
        "a AND (b OR c)",
        "true\n    AND a = 1 -- trailing\n    -- own line\n    AND b = 2",
        "true\n    {% if x -%}\n    AND a = {{ x }}\n    {% endif -%}\n    AND b",
    ]));
}

#[test]
fn function_calls() {
    insta::assert_snapshot!(trees(&[
        "f()",
        "count(*)",
        "count(DISTINCT a, b)",
        "f(DISTINCT)",
        "f(ALL, x)",
        "quantiles(0.5, 0.9)(x)",
        "f(a AS b, c)",
        "db.f(x)",
        "countIf(x) FILTER (WHERE y > 1)",
        "first_value(x) RESPECT NULLS",
        "sum(x) OVER (PARTITION BY y ORDER BY z)",
        "sum(x) OVER w",
        "left(s, 1)",
        "any(x)",
        "format('{}', x)",
        "if(a, b, c)",
        "not(x)",
        "{{ macro }}(x)",
        "f(a,)",
    ]));
}

#[test]
fn special_functions() {
    insta::assert_snapshot!(trees(&[
        "CAST(x AS String)",
        "cast(x, 'String')",
        "CAST(x AS y AS String)",
        "CAST([1, 2] AS Array(Decimal(10, 2)))",
        "try_cast(x AS Int8)",
        "EXTRACT(DAY FROM d)",
        "extract(s, '\\\\d+')",
        "SUBSTRING(s FROM 2 FOR 3)",
        "substring(s, 2, 3)",
        "TRIM(BOTH ' ' FROM s)",
        "TRIM(LEADING FROM s)",
        "trim(s)",
        "POSITION('a' IN s)",
        "position(s, 'a')",
        "OVERLAY(s PLACING 'x' FROM 2 FOR 1)",
        "dateDiff(day, a, b)",
        "dateDiff('day', a, b)",
        "DATE_ADD(HOUR, 1, d)",
        "exists(SELECT 1)",
        "cast('1234' lhs AS UInt32)",
        "substring('1234' AS lhs FROM 2 FOR 2 AS n)",
        "position('a' AS h IN s)",
        "trim(LEADING 'a' AS x FROM s y)",
        "dateAdd(DAY, 1 n, d)",
    ]));
}

#[test]
fn lambdas() {
    insta::assert_snapshot!(trees(&[
        "x -> x + 1",
        "(x, y) -> x + y",
        "arrayMap(x -> x * 2, arr)",
        "arrayFilter((x, i) -> i > 1, arr, idx)",
        "() -> 1",
        "(a + 1) -> b",
    ]));
}

#[test]
fn collections() {
    insta::assert_snapshot!(trees(&[
        "(1, 2)",
        "(1)",
        "()",
        "(a,)",
        "[1, 2]",
        "[]",
        "ARRAY[1, 2]",
        "array[1]",
        "array(1, 2)",
        "{'a': 1, 'b': 2}",
        "tuple(1, 2)",
        "map('a', 1)",
    ]));
}

#[test]
fn element_access() {
    insta::assert_snapshot!(trees(&[
        "a[1]",
        "m['k'][2]",
        "x.1",
        "a.1.2",
        "f(x).1",
        "f(x).name",
        "t.*",
        "db.t.*",
        "t.COLUMNS('re')",
        "json.a.b[]",
        "(1, 2).-1",
        "t.(1 AS a)",
        "{f:Identifier}(x)",
    ]));
}

#[test]
fn case_and_interval() {
    insta::assert_snapshot!(trees(&[
        "CASE WHEN a THEN b END",
        "CASE x WHEN 1 THEN 'a' WHEN 2 THEN 'b' ELSE 'c' END",
        "INTERVAL 1 DAY",
        "INTERVAL '1 day'",
        "INTERVAL '1' DAY",
        "INTERVAL 1 + 1 HOUR",
        "INTERVAL '1-2' YEAR TO MONTH",
        "toStartOfInterval(t, INTERVAL 1 HOUR)",
        "interval",
        "case",
    ]));
}

#[test]
fn column_matchers() {
    insta::assert_snapshot!(trees(&[
        "*",
        "* EXCEPT (a, b)",
        "* EXCEPT a",
        "* EXCEPT STRICT a",
        "* REPLACE (a + 1 AS a)",
        "* APPLY toString",
        "* APPLY(x -> x + 1)",
        "* LIKE 'a%'",
        "COLUMNS('a.*')",
        "COLUMNS(a, b) APPLY sum",
        "t.* EXCEPT (a)",
    ]));
}

/// `EXCEPT` takes names or a pattern, `APPLY` a lambda or a function's
/// name, so words that are literals elsewhere are names here.
#[test]
fn column_transformer_operands() {
    insta::assert_snapshot!(trees(&[
        "* EXCEPT null",
        "* EXCEPT (null, true)",
        "* EXCEPT ('a.*')",
        "* APPLY inf",
        "* APPLY(nan)",
        "* APPLY quantile(0.5)",
        "* APPLY(x -> x + 1, 'p_')",
        "* APPLY((x) -> x)",
        "* REPLACE (null AS a, 2 AS b)",
        "* REPLACE STRICT x + 1 AS x",
    ]));
}

/// `COLUMNS(…)` is a matcher only with a string or names in its brackets;
/// otherwise it calls a function named `columns`.
#[test]
fn columns_matcher_or_function() {
    insta::assert_snapshot!(trees(&[
        "columns(null)",
        "COLUMNS(t.a, {c:Identifier})",
        "t.COLUMNS(a)",
        "columns(a + 1)",
        "COLUMNS()",
        "columns('a', 'b')",
        "t.COLUMNS(1)",
        "t.columns('a')",
    ]));
}

#[test]
fn casts_and_parameters() {
    insta::assert_snapshot!(trees(&[
        "x::Int8",
        "x::Nullable(String)",
        "[1,2]::Array(Int8)",
        "[1,\n2]::Array(Int8)",
        "'1'::Int8",
        "-1::Int8",
        "(1, 'a')::Tuple(Int8, String)",
        "a.1::Int8",
        "{x:UInt32}",
        "{s: String}",
        "@@version",
    ]));
}

#[test]
fn aliases() {
    insta::assert_snapshot!(trees(&[
        "1 AS limit",
        "x AS \"y\"",
        "x y",
        "x \"y\"",
        "x {{ name }}",
        "from AS x",
        "1 limit",
        "from x",
        "f(a + b AS x * 2)",
    ]));
}

#[test]
fn subqueries_and_quantifiers() {
    insta::assert_snapshot!(trees(&[
        "x IN (SELECT 1)",
        "x = ANY (SELECT 1)",
        "x = any(y)",
        "x = SOME([1, 2])",
        "x < ALL (SELECT y)",
        "(SELECT 1) + 1",
        "(WITH 1 AS y SELECT y)",
        "1 IN (FROM t)",
        "(from + 1)",
    ]));
}

#[test]
fn failures_stay_inside_the_smallest_list_item() {
    insta::assert_snapshot!(trees(&[
        "f(a b, c)",
        "(a b, c)",
        "[1, 2 3]",
        "f(g(x y), z)",
        "x ->",
        "a +",
    ]));
}

#[test]
fn in_is_an_operator_again_after_a_failed_position() {
    for source in [
        "f(position(+), x IN (1))",
        "[position(+), x IN (1)]",
        "(position(+), x IN (1))",
    ] {
        let parse = crate::syntax::parse_as(source, crate::lexer::Mode::Sql, Entry::Expression);
        let verbatim: Vec<&str> = parse
            .fallbacks()
            .iter()
            .map(|f| &source[f.range.clone()])
            .collect();
        assert_eq!(verbatim, ["position(+)"], "{source}");
    }
}

/// `input` and how the parser groups it, written in ClickHouse syntax with
/// every operator in parentheses.
const PRECEDENCE: &[(&str, &str)] = &[
    ("a + b * c", "(a + (b * c))"),
    ("a - b - c", "((a - b) - c)"),
    ("NOT a = b", "(NOT (a = b))"),
    ("NOT a AND b", "((NOT a) AND b)"),
    ("NOT a IS NULL", "(NOT (a IS NULL))"),
    ("NOT NOT a", "(NOT (NOT a))"),
    ("a BETWEEN b AND c AND d", "((a BETWEEN b AND c) AND d)"),
    (
        "a BETWEEN b + 1 AND c OR d",
        "((a BETWEEN (b + 1) AND c) OR d)",
    ),
    ("a < b BETWEEN c AND d", "((a < b) BETWEEN c AND d)"),
    ("a BETWEEN 1 AND 2 IS NULL", "((a BETWEEN 1 AND 2) IS NULL)"),
    ("a BETWEEN b = 1 AND c", "(a BETWEEN (b = 1) AND c)"),
    ("a BETWEEN 1 AND 2 = x", "(a BETWEEN 1 AND (2 = x))"),
    ("-x::Int8", "(- (x :: Int8))"),
    ("-1::Int8", "(-1 :: Int8)"),
    ("-x.1", "(- (x . 1))"),
    ("-a * b", "((- a) * b)"),
    ("- - a", "(- (- a))"),
    ("a.1.2", "((a . 1) . 2)"),
    ("x.1::Int8", "((x . 1) :: Int8)"),
    ("a NOT IN (1, 2)", "(a NOT IN ( 1 , 2 ))"),
    ("a = b IS NULL", "((a = b) IS NULL)"),
    ("a IS NULL = b", "((a IS NULL) = b)"),
    ("a = b = c", "((a = b) = c)"),
    ("a LIKE b = c", "((a LIKE b) = c)"),
    ("a || b + c", "(a || (b + c))"),
    ("a + b || c", "((a + b) || c)"),
    ("a AND NOT b OR c", "((a AND (NOT b)) OR c)"),
    ("a OR b AND c", "(a OR (b AND c))"),
    ("a OR b ? c : d", "((a OR b) ? c : d)"),
    ("a ? b OR c : d", "(a ? (b OR c) : d)"),
    ("INTERVAL 1 DAY + x", "((INTERVAL 1 DAY) + x)"),
    ("INTERVAL 1 + 1 DAY", "(INTERVAL (1 + 1) DAY)"),
    ("x AT TIME ZONE 'UTC' + 1", "((x AT TIME ZONE 'UTC') + 1)"),
    ("-x AT TIME ZONE 'UTC'", "((- x) AT TIME ZONE 'UTC')"),
    ("a | b = c", "(a | (b = c))"),
    ("a = b | c", "((a = b) | c)"),
    (
        "a IS NOT DISTINCT FROM b = c",
        "(a IS NOT DISTINCT FROM (b = c))",
    ),
    ("arrayMap(x -> x + 1, a)", "arrayMap ( (x -> (x + 1)) , a )"),
    ("a[1].2", "((a [ 1 ]) . 2)"),
    (
        "a LIKE 'x' || 'y' ESCAPE '!'",
        "(a LIKE ('x' || 'y') ESCAPE '!')",
    ),
    (
        "x = 1 AND y = 2 OR z = 3",
        "(((x = 1) AND (y = 2)) OR (z = 3))",
    ),
    ("f(a + b AS x * 2)", "f ( ((a + b) AS x * 2) )"),
    ("(1, 2).-1", "(( 1 , 2 ) . - 1)"),
    ("x < some(y, z) + 1", "(x < (some ( y , z ) + 1))"),
    ("x = some(y) + 1", "((x = some ( y )) + 1)"),
];

#[test]
fn precedence() {
    for (input, expected) in PRECEDENCE {
        assert_eq!(&parenthesized(input), expected, "{input}");
    }
}

/// Expressions ClickHouse rejects, and how chairlift reads them anyway.
#[test]
fn precedence_beyond_clickhouse() {
    // ClickHouse needs parentheses around a nested `?:` and a curried lambda.
    assert_eq!(parenthesized("a ? b : c ? d : e"), "((a ? b : c) ? d : e)");
    assert_eq!(parenthesized("x -> (y -> z)"), "(x -> ( (y -> z) ))");
    // ClickHouse leaves the `?` of a finished `?:` on its operator stack
    // when an operator of `OR`'s priority or lower follows, and then fails.
    assert_eq!(parenthesized("a ? b : c OR d"), "((a ? b : c) OR d)");
}

/// The precedence table against ClickHouse: `EXPLAIN AST` of each input and
/// of its parenthesised form must be the same, which they are only if the
/// parser groups operands as ClickHouse does.
#[test]
fn precedence_agrees_with_clickhouse() {
    const MARKER: &str = "chairlift-boundary";
    let Some(clickhouse) = clickhouse_local() else {
        return;
    };
    // One process for all, with a marker after each `EXPLAIN`; every
    // expression in the table is valid, so none stops the run.
    let mut query = String::new();
    for expr in PRECEDENCE
        .iter()
        .flat_map(|(input, expected)| [input, expected])
    {
        let _ = writeln!(query, "EXPLAIN AST SELECT {expr}; SELECT '{MARKER}';");
    }
    let output = std::process::Command::new(&clickhouse)
        .args(["local", "--multiquery", "--query", &query])
        .output()
        .expect("clickhouse local runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8");
    let asts: Vec<&str> = stdout.split(&format!("{MARKER}\n")).collect();
    let mut differences = Vec::new();
    for (pair, (input, expected)) in asts.chunks(2).zip(PRECEDENCE) {
        if pair.len() < 2 || pair[0] != pair[1] {
            differences.push(format!("{input} is not {expected}:\n{}", pair.join("\n")));
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

/// `clickhouse`, for `clickhouse local`, if it runs; `CHAIRLIFT_CLICKHOUSE`
/// names the binary.
fn clickhouse_local() -> Option<std::ffi::OsString> {
    let program = std::env::var_os("CHAIRLIFT_CLICKHOUSE").unwrap_or_else(|| "clickhouse".into());
    let runs = std::process::Command::new(&program)
        .args(["local", "--query", "SELECT 1"])
        .output()
        .is_ok_and(|o| o.status.success());
    if runs {
        return Some(program);
    }
    assert!(
        std::env::var_os("CHAIRLIFT_REQUIRE_CORPUS").is_none_or(|v| v.is_empty()),
        "clickhouse local is unavailable, and CHAIRLIFT_REQUIRE_CORPUS is set"
    );
    eprintln!("skipping: clickhouse local is unavailable");
    None
}

/// Bare words and what the parser makes of them: each input, and the input
/// with its keyword slots in canonical case. Names keep their case.
const KEYWORD_SLOTS: &[(&str, &str)] = &[
    ("left(s, 1)", "left(s, 1)"),
    ("any(x)", "any(x)"),
    ("format('{}', x)", "format('{}', x)"),
    ("if(a, b, c)", "if(a, b, c)"),
    ("1 as limit", "1 AS limit"),
    ("1 as from", "1 AS from"),
    ("null", "NULL"),
    ("Null", "NULL"),
    ("isNull(x)", "isNull(x)"),
    ("TRUE", "true"),
    ("Inf", "inf"),
    ("NaN", "nan"),
    ("key", "key"),
    ("value + type", "value + type"),
    ("date", "date"),
    ("table", "table"),
    ("settings", "settings"),
    ("t.from", "t.from"),
    ("t.select", "t.select"),
    ("and", "and"),
    ("a and b or not c", "a AND b OR NOT c"),
    ("x not in (1)", "x NOT IN (1)"),
    ("x global in t", "x GLOBAL IN t"),
    ("x is not null", "x IS NOT NULL"),
    ("x between 1 and 2", "x BETWEEN 1 AND 2"),
    (
        "case when a then b else c end",
        "CASE WHEN a THEN b ELSE c END",
    ),
    ("interval 1 day", "INTERVAL 1 DAY"),
    ("interval", "interval"),
    ("interval + 1", "interval + 1"),
    ("case", "case"),
    ("cast(x as String)", "cast(x AS String)"),
    ("cast(x, 'String')", "cast(x, 'String')"),
    ("extract(day from d)", "extract(DAY FROM d)"),
    ("extract(s, 'a')", "extract(s, 'a')"),
    ("count(distinct x)", "count(DISTINCT x)"),
    ("f(distinct)", "f(distinct)"),
    ("date '2026-01-01'", "DATE '2026-01-01'"),
    ("date(x)", "date(x)"),
    ("x like 'a' escape 'b'", "x LIKE 'a' ESCAPE 'b'"),
    ("dateDiff(day, a, b)", "dateDiff(DAY, a, b)"),
    ("columns('a')", "COLUMNS('a')"),
    ("columns(null, true)", "COLUMNS(null, true)"),
    ("columns(null + 1)", "columns(NULL + 1)"),
    ("columns(a, 'b')", "columns(a, 'b')"),
    ("columns()", "columns()"),
    ("* except (a)", "* EXCEPT (a)"),
    ("* except (null, true)", "* EXCEPT (null, true)"),
    ("* except strict nan", "* EXCEPT STRICT nan"),
    ("* apply false", "* APPLY false"),
    ("* apply(Inf) apply(x -> x)", "* APPLY(Inf) APPLY(x -> x)"),
    ("* replace (null as x)", "* REPLACE (NULL AS x)"),
    ("not(x)", "NOT(x)"),
    ("array(1, 2)", "array(1, 2)"),
    ("array[1, 2]", "ARRAY[1, 2]"),
    ("x::Nullable(String)", "x::Nullable(String)"),
    ("x::nullable(string)", "x::nullable(string)"),
    (
        "x::AggregateFunction(1, null, String)",
        "x::AggregateFunction(1, null, String)",
    ),
    (
        "x::SimpleAggregateFunction(true, Nullable(String))",
        "x::SimpleAggregateFunction(true, Nullable(String))",
    ),
    ("{x:Int8}", "{x:Int8}"),
    (
        "sum(x) over (partition by y)",
        "sum(x) OVER (partition by y)",
    ),
    ("x is TRUE", "x IS true"),
    ("position(a in b)", "position(a IN b)"),
    ("trim(both 'x' from s)", "trim(BOTH 'x' FROM s)"),
    ("x at time zone 'UTC'", "x AT TIME ZONE 'UTC'"),
    ("x mod 2", "x MOD 2"),
    ("{{ x }} and y", "{{ x }} AND y"),
    ("events_{{ s }}", "events_{{ s }}"),
    ("'{{ and }}' and y", "'{{ and }}' AND y"),
    ("f(x) filter (where y)", "f(x) FILTER (WHERE y)"),
    ("x = any (select 1)", "x = ANY (SELECT 1)"),
    ("x = some(y)", "x = SOME(y)"),
    ("x like all(y)", "x LIKE ALL(y)"),
    ("x = some(1, 2)", "x = some(1, 2)"),
    ("x = all()", "x = all()"),
    ("x = all(y as z)", "x = all(y AS z)"),
    ("x = some((select 1), 2)", "x = some((SELECT 1), 2)"),
    ("x = any(y)", "x = any(y)"),
];

#[test]
fn keyword_slots() {
    for (input, expected) in KEYWORD_SLOTS {
        assert_eq!(&recased(input, Entry::Expression), expected, "{input}");
        // The verifier lets each recorded word change only into its
        // canonical case, so a slot on a word that is no keyword fails here.
        let parse = crate::syntax::parse_as(input, crate::lexer::Mode::Jinja, Entry::Expression);
        crate::verify::verify(input, expected, crate::lexer::Mode::Jinja, parse.facts())
            .unwrap_or_else(|e| panic!("{input}: {e}"));
    }
}

/// The keyword slots against ClickHouse: `clickhouse-format` must print the
/// re-cased expression as it prints the input, which it does not if a name
/// was re-cased as a keyword.
#[cfg(feature = "oracle")]
#[test]
fn keyword_slots_agree_with_clickhouse() {
    let oracle = crate::oracle::Oracle::new(
        std::env::var_os("CHAIRLIFT_CLICKHOUSE_FORMAT")
            .unwrap_or_else(|| "clickhouse-format".into()),
    );
    if !oracle.is_available() {
        assert!(
            std::env::var_os("CHAIRLIFT_REQUIRE_CORPUS").is_none_or(|v| v.is_empty()),
            "clickhouse-format is unavailable, and CHAIRLIFT_REQUIRE_CORPUS is set"
        );
        eprintln!("skipping: clickhouse-format is unavailable");
        return;
    }
    let texts: Vec<String> = KEYWORD_SLOTS
        .iter()
        .filter(|(input, _)| !input.contains("{{"))
        .flat_map(|(input, expected)| [format!("SELECT {input}"), format!("SELECT {expected}")])
        .collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let verdicts = oracle.canonical(&refs);
    for (pair, texts) in verdicts.chunks(2).zip(texts.chunks(2)) {
        assert_eq!(
            crate::oracle::judge(&pair[0], &pair[1]),
            crate::oracle::Judgement::Same,
            "{} vs {}",
            texts[0],
            texts[1]
        );
    }
}

#[test]
fn deep_nesting_becomes_verbatim_without_overflowing() {
    let deep = 20_000;
    for (open, close) in [
        ("(", ")"),
        ("f(", ")"),
        ("[", "]"),
        ("NOT ", ""),
        ("- ", ""),
        ("x -> ", ""),
        ("CASE WHEN ", " THEN 1 END"),
        ("INTERVAL ", " DAY"),
        ("a ? ", " : b"),
    ] {
        let source = format!("{}x{}", open.repeat(deep), close.repeat(deep));
        // A thread with the default stack of a test thread, so that the
        // depth limit is what keeps the stack from overflowing.
        let tree = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || {
                let parse =
                    crate::syntax::parse_as(&source, crate::lexer::Mode::Sql, Entry::Expression);
                assert_eq!(parse.syntax().text().to_string(), source);
                parse
                    .syntax()
                    .descendants()
                    .any(|n| n.kind().fallback_level().is_some())
            })
            .expect("thread starts")
            .join()
            .expect("no overflow");
        assert!(tree, "{open}…{close} keeps a verbatim region");
    }
}

#[test]
fn long_operator_chains_are_bounded() {
    let source = vec!["a"; 5000].join(" + ");
    let parse = crate::syntax::parse_as(&source, crate::lexer::Mode::Sql, Entry::Expression);
    assert_eq!(parse.syntax().text().to_string(), source);
    let and_chain = vec!["a"; 5000].join(" AND ");
    let parse = crate::syntax::parse_as(&and_chain, crate::lexer::Mode::Sql, Entry::Expression);
    // A run of `AND` is one node however long, so it parses.
    assert_eq!(parse.fallbacks(), &[]);
}
