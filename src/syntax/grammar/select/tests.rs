//! `SELECT` queries: tree snapshots per construct, and the keyword slots.

use std::fmt::Write as _;

use crate::syntax::Entry;
use crate::syntax::FallbackLevel;
use crate::syntax::StatementOutcome;
use crate::syntax::test_support::recased;
use crate::syntax::test_support::tree;

/// The trees of `inputs`, each under its input, for one snapshot.
fn trees(inputs: &[&str]) -> String {
    let mut out = String::new();
    for input in inputs {
        out.push_str("> ");
        out.push_str(input);
        out.push('\n');
        out.push_str(&tree(input, Entry::File));
        out.push('\n');
    }
    out
}

/// The fallbacks of `source`, by level.
fn fallbacks(source: &str) -> Vec<FallbackLevel> {
    crate::syntax::parse(source, crate::lexer::Mode::Jinja)
        .fallbacks()
        .iter()
        .map(|f| f.level)
        .collect()
}

#[test]
fn clauses_of_a_query() {
    insta::assert_snapshot!(trees(&[
        "SELECT a, b FROM t PREWHERE p WHERE w GROUP BY a HAVING h QUALIFY q ORDER BY a LIMIT 10",
        "SELECT 1",
        "SELECT x FROM t SETTINGS max_threads = 8 FORMAT JSONEachRow",
        "SELECT x FROM t INTO OUTFILE 'out.csv' AND STDOUT COMPRESSION 'gzip' FORMAT CSV",
        "SELECT x FROM t WINDOW w AS (PARTITION BY y ORDER BY z), v AS (w ROWS 1 PRECEDING)",
        "SELECT 1 SETTINGS a = 1 FORMAT JSON SETTINGS b = 2",
    ]));
    assert_eq!(
        fallbacks("SELECT 1 INTO OUTFILE 'x' TRUNCATE COMPRESSION 'zstd' LEVEL 3 FORMAT CSV"),
        []
    );
    assert_eq!(
        fallbacks("SELECT 1 INTO OUTFILE 'x' foo bar"),
        [FallbackLevel::Clause]
    );
}

#[test]
fn with_clauses() {
    insta::assert_snapshot!(trees(&[
        "WITH daily AS (SELECT 1 AS x) SELECT x FROM daily",
        "WITH a AS (SELECT 1), b AS (SELECT 2) SELECT * FROM a, b",
        "WITH 1 AS one, now() AS t, (SELECT max(x) FROM u) AS m SELECT one",
        "WITH RECURSIVE r AS (SELECT 1 AS n UNION ALL SELECT n + 1 FROM r WHERE n < 3) SELECT n FROM r",
        "WITH m AS MATERIALIZED (SELECT 1) SELECT * FROM m",
        "WITH c (a, b) AS (SELECT 1, 2) SELECT a FROM c",
        "WITH x AS materialized SELECT materialized",
        "WITH a AS (SELECT 1), SELECT * FROM a",
        "WITH 1 AS x FROM t",
        // Run together, the branches read as ClickHouse would read them: the
        // first branch's table takes the second's `select` as its alias.
        "WITH x AS (SELECT 1)\n{% if a %}\nSELECT a FROM x\n{% else %}\nSELECT b FROM x\n{% endif %}",
    ]));
    // ClickHouse reads no query without `SELECT` or `FROM` after `WITH`.
    assert_eq!(fallbacks("WITH 1 AS x"), [FallbackLevel::Statement]);
}

#[test]
fn select_clauses() {
    insta::assert_snapshot!(trees(&[
        "SELECT DISTINCT a, b",
        "SELECT DISTINCT ON (a, b) a, c",
        "SELECT ALL a",
        "SELECT TOP 10 a FROM t ORDER BY a",
        "SELECT TOP (5) WITH TIES a FROM t ORDER BY a",
        "SELECT a x, b AS y, c \"z\"",
        "SELECT * EXCEPT (raw), t.* REPLACE (a + 1 AS a), COLUMNS('^x') APPLY sum FROM t",
        "SELECT a, b, FROM t",
        "SELECT a, from FROM t",
    ]));
}

#[test]
fn tables() {
    insta::assert_snapshot!(trees(&[
        "SELECT * FROM db.t AS x FINAL SAMPLE 1 / 10 OFFSET 1 / 2",
        "SELECT * FROM t x",
        "SELECT * FROM (SELECT 1) AS daily",
        "SELECT * FROM numbers(10) n",
        "SELECT * FROM remote('host', db.t), cluster('c', view(SELECT 1))",
        "SELECT * FROM view(SELECT a FROM t UNION ALL SELECT b FROM u)",
        "SELECT * FROM viewIfPermitted(SELECT 1 ELSE null('x UInt8'))",
        "SELECT * FROM file('a.csv', SETTINGS format_csv_delimiter = ';', input_format_csv_skip_first_lines = 1)",
        "SELECT * FROM VALUES('a UInt8', 1, 2)",
        "SELECT * FROM 'data.csv'",
        "SELECT * FROM t AS x (a, b)",
        "SELECT * FROM t STREAM BOUNDED WATERMARK FOR ts AS ts - INTERVAL 10 SECOND",
        "SELECT * FROM t STREAM CURSOR {'0': {'block_number': 10, 'block_offset': 1}}",
        "SELECT * FROM {db:Identifier}.{t:Identifier}",
        "SELECT * FROM {{ ref('events') }} AS e",
        "SELECT * FROM (a JOIN b ON a.x = b.x) AS j",
    ]));
}

#[test]
fn joins() {
    insta::assert_snapshot!(trees(&[
        "SELECT * FROM a JOIN b ON a.x = b.x AND a.y = b.y",
        "SELECT * FROM a LEFT JOIN b USING (x, y) INNER JOIN c USING x",
        "SELECT * FROM a GLOBAL ANY LEFT OUTER JOIN b ON 1 ASOF LEFT JOIN c ON 1",
        "SELECT * FROM a LEFT ANY JOIN b USING x SEMI LEFT JOIN c USING x RIGHT ANTI JOIN d USING x",
        "SELECT * FROM a CROSS JOIN b PASTE JOIN c NATURAL JOIN d",
        "SELECT * FROM a FULL OUTER JOIN b ON 1, c",
        "SELECT * FROM a, b ARRAY JOIN arr AS e, [1, 2] AS n LEFT ARRAY JOIN m",
        "SELECT * FROM a JOIN LATERAL (SELECT 1) AS l ON true",
        "SELECT * FROM a AS x LOCAL JOIN b ON 1",
    ]));
}

#[test]
fn grouping() {
    insta::assert_snapshot!(trees(&[
        "SELECT a, count() FROM t GROUP BY a WITH TOTALS",
        "SELECT a FROM t GROUP BY ALL",
        "SELECT a FROM t GROUP BY ROLLUP(a, b) WITH CUBE WITH TOTALS",
        "SELECT a FROM t GROUP BY CUBE(a)",
        "SELECT a FROM t GROUP BY GROUPING SETS ((a, b), a, ())",
        "SELECT count() FROM t WITH TOTALS",
    ]));
}

#[test]
fn order_and_limit() {
    insta::assert_snapshot!(trees(&[
        "SELECT a FROM t ORDER BY a DESC NULLS FIRST, b COLLATE 'en' LIMIT 1, 10",
        "SELECT a FROM t ORDER BY ALL DESC",
        "SELECT a FROM t ORDER BY all, b",
        "SELECT d FROM t ORDER BY d WITH FILL FROM 1 TO 10 STEP 1 INTERPOLATE (x AS x + 1, y)",
        "SELECT a FROM t ORDER BY a LIMIT 3 BY a LIMIT 10 OFFSET 5 WITH TIES",
        "SELECT a FROM t LIMIT 2 OFFSET 1 BY a, b",
        "SELECT a FROM t ORDER BY a OFFSET 5 ROWS FETCH NEXT 3 ROWS ONLY",
        "SELECT a FROM t LIMIT AFTER a > 1 ALL UNTIL a > 5",
        "SELECT a FROM t LIMIT after BY a",
    ]));
}

#[test]
fn set_operations() {
    insta::assert_snapshot!(trees(&[
        "SELECT 1 UNION ALL SELECT 2 UNION DISTINCT SELECT 3",
        "SELECT 1 INTERSECT SELECT 2 EXCEPT ALL SELECT 3",
        "(SELECT 1) UNION ALL (SELECT 2 ORDER BY 1 LIMIT 1) FORMAT JSON",
        "(SELECT 1)",
        "SELECT 1 FROM t UNION ALL WITH 2 AS x SELECT x FORMAT TSV",
        // The output clauses are the statement's, but a `SETTINGS` right
        // after a query without brackets.
        "SELECT 1 UNION ALL SELECT 2 SETTINGS a = 1 FORMAT JSON SETTINGS b = 2",
        "(SELECT 1) SETTINGS a = 1",
    ]));
    assert_eq!(
        fallbacks("(SELECT 1) ORDER BY 1"),
        [FallbackLevel::Statement]
    );
    assert_eq!(
        fallbacks("SELECT * FROM (SELECT 1 FORMAT JSON)"),
        [FallbackLevel::Item]
    );
}

#[test]
fn subqueries_in_expressions() {
    insta::assert_snapshot!(trees(&[
        "SELECT (SELECT max(x) FROM t) AS m",
        "SELECT * FROM t WHERE x IN (SELECT x FROM u) AND exists(SELECT 1 FROM v)",
        "SELECT * FROM t WHERE x = ANY (SELECT 1 UNION ALL SELECT 2)",
        "SELECT * FROM t WHERE x GLOBAL NOT IN (FROM u SELECT x)",
    ]));
}

#[test]
fn from_first() {
    insta::assert_snapshot!(trees(&[
        "FROM t SELECT x WHERE y",
        "FROM t",
        "FROM t AS s SELECT s.x",
        "SELECT * FROM (FROM t)",
    ]));
}

#[test]
fn clauses_in_any_order_and_repeated() {
    insta::assert_snapshot!(trees(&[
        "SELECT a\nFROM t\n{% if x %}\nWHERE a = 1\n{% else %}\nWHERE a = 2\n{% endif %}\nORDER BY a",
        "SELECT a\nFROM t\n-- a comment\nWHERE a\n/* another */\nLIMIT 1;",
        "SELECT a FROM t WHERE a b c GROUP BY a",
        "SELECT a, f(x y), b FROM t",
        "SELECT a FROM t LEFT JOIN u ON a = garbage garbage JOIN v USING (x)",
    ]));
    // Each failure is the smallest region that works.
    assert_eq!(
        fallbacks("SELECT a FROM t WHERE a b c GROUP BY a"),
        [FallbackLevel::Clause]
    );
    assert_eq!(
        fallbacks("SELECT a, f(x y), b FROM t"),
        [FallbackLevel::Item]
    );
    assert_eq!(
        fallbacks("FROM t |> WHERE x |> SELECT y"),
        [FallbackLevel::Statement]
    );
    assert_eq!(fallbacks("SELECT * FROM t SETTINGS final, a = 1"), []);
}

#[test]
fn design_example_parses_without_fallback() {
    let source = include_str!("../../../../tests/fixtures/design/page_views_1h.sql.j2");
    let start = source.find("select").expect("a SELECT");
    let end = start + source[start..].find(';').expect("its end");
    let query = &source[start..end];
    let parse = crate::syntax::parse(query, crate::lexer::Mode::Jinja);
    assert_eq!(parse.fallbacks(), &[]);
    assert_eq!(parse.statements()[0].outcome, StatementOutcome::Parsed);
}

/// Words that are keywords in one place and names in another: each input,
/// and the input with its keyword slots in canonical case. Names keep their
/// case.
const KEYWORD_SLOTS: &[(&str, &str)] = &[
    ("select Settings from t", "SELECT Settings FROM t"),
    ("select a, settings from t", "SELECT a, settings FROM t"),
    (
        "select 1 from t settings max_threads = 1",
        "SELECT 1 FROM t SETTINGS max_threads = 1",
    ),
    (
        "select 1 from t order by settings",
        "SELECT 1 FROM t ORDER BY settings",
    ),
    ("select * from union", "SELECT * FROM union"),
    (
        "select * from union union all select 1",
        "SELECT * FROM union UNION ALL SELECT 1",
    ),
    ("select interval from t", "SELECT interval FROM t"),
    (
        "select left(s, 1), any(x), format('{}', y) from t",
        "SELECT left(s, 1), any(x), format('{}', y) FROM t",
    ),
    (
        "select * from a left join b on 1 any join c on 1",
        "SELECT * FROM a LEFT JOIN b ON 1 ANY JOIN c ON 1",
    ),
    ("select 1 format JSON", "SELECT 1 FORMAT JSON"),
    (
        "select format from format format JSON",
        "SELECT format FROM format FORMAT JSON",
    ),
    ("select 1 from t where (a)", "SELECT 1 FROM t WHERE (a)"),
    ("select 1 as limit", "SELECT 1 AS limit"),
    (
        "select 1 from t group by table",
        "SELECT 1 FROM t GROUP BY table",
    ),
    ("select final from t final", "SELECT final FROM t FINAL"),
    (
        "select sample from sample sample 1/2 offset 1/2",
        "SELECT sample FROM sample SAMPLE 1/2 OFFSET 1/2",
    ),
    (
        "select offset from offset limit 1 offset 2",
        "SELECT offset FROM offset LIMIT 1 OFFSET 2",
    ),
    (
        "select limit from limit limit limit",
        "SELECT limit FROM limit LIMIT limit",
    ),
    ("select a, limit from t", "SELECT a, limit FROM t"),
    (
        "select array from t array join array",
        "SELECT array FROM t ARRAY JOIN array",
    ),
    ("select a from all", "SELECT a FROM all"),
    ("select with from with", "SELECT with FROM with"),
    ("select window from window", "SELECT window FROM window"),
    (
        "select 1 from t window w as (order by x)",
        "SELECT 1 FROM t WINDOW w AS (order by x)",
    ),
    (
        "select fill from t order by fill",
        "SELECT fill FROM t ORDER BY fill",
    ),
    (
        "select d from t order by d with fill",
        "SELECT d FROM t ORDER BY d WITH FILL",
    ),
    ("select totals from totals", "SELECT totals FROM totals"),
    ("with totals select 1", "WITH totals SELECT 1"),
    (
        "select count() from t with totals",
        "SELECT count() FROM t WITH TOTALS",
    ),
    ("select a as b from t as c", "SELECT a AS b FROM t AS c"),
    ("select a b from t c", "SELECT a b FROM t c"),
    (
        "select * from t local join u on 1",
        "SELECT * FROM t local JOIN u ON 1",
    ),
    (
        "select * from a join lateral on 1",
        "SELECT * FROM a JOIN lateral ON 1",
    ),
    (
        "select * from t prewhere a qualify b",
        "SELECT * FROM t PREWHERE a QUALIFY b",
    ),
    ("select 1 from t limit after", "SELECT 1 FROM t LIMIT after"),
    ("select top from t", "SELECT top FROM t"),
    (
        "select distinct on (a) a from t",
        "SELECT DISTINCT ON (a) a FROM t",
    ),
    (
        "with x as (select 1) select * from x",
        "WITH x AS (SELECT 1) SELECT * FROM x",
    ),
    ("from t select x", "FROM t SELECT x"),
    // `select` after a table is its alias but where it starts the `SELECT`
    // clause of the `FROM`-first form.
    (
        "select * from t select final",
        "SELECT * FROM t select FINAL",
    ),
    (
        "select * from t select where 1",
        "SELECT * FROM t select WHERE 1",
    ),
    ("select * from t select", "SELECT * FROM t select"),
    (
        "select * from t select join u on 1",
        "SELECT * FROM t select JOIN u ON 1",
    ),
    (
        "select 1 from t select except select 1",
        "SELECT 1 FROM t select EXCEPT SELECT 1",
    ),
    ("from t select final", "FROM t select FINAL"),
    ("from t select, u", "FROM t select, u"),
    ("from t select select x", "FROM t select SELECT x"),
    (
        "from a join b select on a.x = select.x",
        "FROM a JOIN b select ON a.x = select.x",
    ),
    ("from t select on", "FROM t SELECT on"),
    (
        "from a cross join b select x",
        "FROM a CROSS JOIN b SELECT x",
    ),
    (
        "select * from t final (final)",
        "SELECT * FROM t FINAL (final)",
    ),
    (
        "select * from (a join b on 1) j",
        "SELECT * FROM (a JOIN b ON 1) j",
    ),
    (
        "select * from t group by all order by all desc",
        "SELECT * FROM t GROUP BY ALL ORDER BY ALL DESC",
    ),
    (
        "select * from t group by rollup(a) with totals",
        "SELECT * FROM t GROUP BY ROLLUP(a) WITH TOTALS",
    ),
    (
        "select * from t group by grouping sets ((a), ())",
        "SELECT * FROM t GROUP BY GROUPING SETS ((a), ())",
    ),
    (
        "select * from t order by a offset 1 rows fetch first 1 rows only",
        "SELECT * FROM t ORDER BY a OFFSET 1 ROWS FETCH FIRST 1 ROWS ONLY",
    ),
    (
        "select * from t order by a limit 1 by a limit 2 with ties",
        "SELECT * FROM t ORDER BY a LIMIT 1 BY a LIMIT 2 WITH TIES",
    ),
    (
        "select * from file('x', settings a = 1)",
        "SELECT * FROM file('x', SETTINGS a = 1)",
    ),
    (
        "select * from view(select 1)",
        "SELECT * FROM view(SELECT 1)",
    ),
    (
        "select * from exists(select 1)",
        "SELECT * FROM exists(SELECT 1)",
    ),
    // `ELSE` ends the query of `viewIfPermitted`, and a table function's
    // name follows it.
    (
        "select * from viewIfPermitted(select 1 from t else null('a UInt8'))",
        "SELECT * FROM viewIfPermitted(SELECT 1 FROM t ELSE null('a UInt8'))",
    ),
    (
        "select * from viewIfPermitted(select 1 else limit(1))",
        "SELECT * FROM viewIfPermitted(SELECT 1 ELSE limit(1))",
    ),
    (
        "select * from viewIfPermitted(select a from t where x else where(1))",
        "SELECT * FROM viewIfPermitted(SELECT a FROM t WHERE x ELSE where(1))",
    ),
    (
        "select 1 except select 2 intersect distinct select 3",
        "SELECT 1 EXCEPT SELECT 2 INTERSECT DISTINCT SELECT 3",
    ),
    ("select * except (a) from t", "SELECT * EXCEPT (a) FROM t"),
    (
        "select x from t into outfile 'f' format CSV",
        "SELECT x FROM t INTO OUTFILE 'f' FORMAT CSV",
    ),
    (
        "select 1 union all select 2 into outfile 'f' append and stdout compression 'gzip' level 1 \
         format CSV settings a = 1",
        "SELECT 1 UNION ALL SELECT 2 INTO OUTFILE 'f' APPEND AND STDOUT COMPRESSION 'gzip' LEVEL 1 \
         FORMAT CSV SETTINGS a = 1",
    ),
    (
        "select * from t settings limit = 5",
        "SELECT * FROM t SETTINGS limit = 5",
    ),
    // A setting's name alone is ClickHouse's shorthand for `= true`.
    (
        "select * from t settings final",
        "SELECT * FROM t SETTINGS final",
    ),
    (
        "select 1 settings a = 1, b, limit",
        "SELECT 1 SETTINGS a = 1, b, limit",
    ),
    (
        "select * from remote('h', t, settings x)",
        "SELECT * FROM remote('h', t, SETTINGS x)",
    ),
    ("select 1 from t as settings", "SELECT 1 FROM t AS settings"),
    // `(select …` and `(from …` are subqueries only where ClickHouse's
    // parser reads one.
    (
        "select arrayMap((select, y) -> y, a, b)",
        "SELECT arrayMap((select, y) -> y, a, b)",
    ),
    ("select (select)", "SELECT (select)"),
    ("select (select = 1)", "SELECT (select = 1)"),
    ("select (select and x)", "SELECT (SELECT and x)"),
    ("select (with, y)", "SELECT (with, y)"),
    ("select (with - 1)", "SELECT (with - 1)"),
    ("select (with [1])", "SELECT (with [1])"),
    ("select (with in (1))", "SELECT (with IN (1))"),
    ("select (with and x)", "SELECT (with AND x)"),
    ("select (with is null)", "SELECT (with IS NULL)"),
    ("select x in (with - 1)", "SELECT x IN (with - 1)"),
    (
        "select (with 1 as x select x)",
        "SELECT (WITH 1 AS x SELECT x)",
    ),
    ("with 1 as x from t", "WITH 1 AS x FROM t"),
    ("with 1 as x", "with 1 as x"),
    ("select (from and x)", "SELECT (from AND x)"),
    ("select (from in t)", "SELECT (from IN t)"),
    ("select (from is null)", "SELECT (from IS NULL)"),
    ("select (from like 'a')", "SELECT (from LIKE 'a')"),
    (
        "select (from between 0 and 2)",
        "SELECT (from BETWEEN 0 AND 2)",
    ),
    ("select * from (from t)", "SELECT * FROM (FROM t)"),
    // `FROM` starts a query only before a table, and brackets after it hold
    // one only if they hold a query or joins.
    ("select (from (1))", "SELECT (from (1))"),
    ("select (from (x))", "SELECT (from (x))"),
    ("select (from (x, y))", "SELECT (FROM (x, y))"),
    ("from and select 1", "FROM and SELECT 1"),
    ("from in select 1", "FROM in SELECT 1"),
    ("from 'f.csv' select 1", "FROM 'f.csv' SELECT 1"),
    ("create table t as from(1)", "CREATE TABLE t AS from(1)"),
    // `ALL` and `DISTINCT` right after `SELECT` are always modifiers.
    ("select all in (1)", "SELECT ALL in (1)"),
    ("select all is null", "SELECT ALL is null"),
    ("select distinct in (1)", "SELECT DISTINCT in (1)"),
    // Expressions where ClickHouse reads expressions, names where it reads
    // identifiers.
    (
        "select * from a join b using (null)",
        "SELECT * FROM a JOIN b USING (NULL)",
    ),
    (
        "select * from t array join null",
        "SELECT * FROM t ARRAY JOIN NULL",
    ),
    (
        "with null as (select 1) select * from null",
        "WITH null AS (SELECT 1) SELECT * FROM null",
    ),
    ("select * from t as x (null)", "SELECT * FROM t AS x (null)"),
    (
        "select * from t window null as ()",
        "SELECT * FROM t WINDOW null AS ()",
    ),
    (
        "select x from t order by x with fill interpolate (null)",
        "SELECT x FROM t ORDER BY x WITH FILL INTERPOLATE (null)",
    ),
    ("select * from null.true", "SELECT * FROM null.true"),
    // A projection's query has no modifiers, and `COMMENT 'x'` after the
    // query of `CREATE … AS SELECT` is the statement's.
    (
        "create table t (a Int8, projection p (select distinct, all group by all order by all)) engine = MergeTree order by a",
        "CREATE TABLE t (a Int8, PROJECTION p (SELECT distinct, all GROUP BY all ORDER BY all)) ENGINE = MergeTree ORDER BY a",
    ),
    (
        "create table t (a Int8, projection p (with 1 as x select top, x group by rollup(a), cube)) engine = MergeTree order by a",
        "CREATE TABLE t (a Int8, PROJECTION p (WITH 1 AS x SELECT top, x GROUP BY rollup(a), cube)) ENGINE = MergeTree ORDER BY a",
    ),
    (
        "create table t engine = Memory as select a from u comment 'x'",
        "CREATE TABLE t ENGINE = Memory AS SELECT a FROM u COMMENT 'x'",
    ),
    (
        "create table t engine = Memory as select a comment from u",
        "CREATE TABLE t ENGINE = Memory AS SELECT a comment FROM u",
    ),
    (
        "create materialized view v to t as select a from u where b comment 'x'",
        "CREATE MATERIALIZED VIEW v TO t AS SELECT a FROM u WHERE b COMMENT 'x'",
    ),
    // Pipe syntax is not parsed, so nothing in it is re-cased.
    (
        "from select s |> where id = 2",
        "from select s |> where id = 2",
    ),
];

#[test]
fn keyword_slots() {
    for (input, expected) in KEYWORD_SLOTS {
        assert_eq!(&recased(input, Entry::File), expected, "{input}");
        // The verifier lets each recorded word change only into its
        // canonical case, so a slot on a word that is no keyword fails here.
        let parse = crate::syntax::parse(input, crate::lexer::Mode::Jinja);
        crate::verify::verify(input, expected, crate::lexer::Mode::Jinja, parse.facts())
            .unwrap_or_else(|e| panic!("{input}: {e}"));
    }
}

/// Inputs of [`KEYWORD_SLOTS`] that ClickHouse rejects, so that the oracle
/// cannot judge them.
const REJECTED_BY_CLICKHOUSE: &[&str] = &["select top from t", "with 1 as x"];

/// The keyword slots against ClickHouse: `clickhouse-format` must print the
/// re-cased query as it prints the input, which it does not if a name was
/// re-cased as a keyword.
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
    let pairs: Vec<&(&str, &str)> = KEYWORD_SLOTS
        .iter()
        .filter(|(input, _)| !REJECTED_BY_CLICKHOUSE.contains(input))
        .collect();
    let texts: Vec<&str> = pairs.iter().flat_map(|(a, b)| [*a, *b]).collect();
    let verdicts = oracle.canonical(&texts);
    let mut differences = String::new();
    for (pair, (input, expected)) in verdicts.chunks(2).zip(&pairs) {
        let judgement = crate::oracle::judge(&pair[0], &pair[1]);
        if judgement != crate::oracle::Judgement::Same {
            let _ = writeln!(differences, "{input} vs {expected}: {judgement:?}");
        }
    }
    assert!(differences.is_empty(), "{differences}");
}

#[test]
fn deep_subqueries_become_verbatim_without_overflowing() {
    let deep = 5_000;
    let source = format!("{}1{}", "SELECT * FROM (".repeat(deep), ")".repeat(deep));
    let parsed = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            let parse = crate::syntax::parse(&source, crate::lexer::Mode::Sql);
            assert_eq!(parse.syntax().text().to_string(), source);
        })
        .expect("thread starts")
        .join();
    assert!(parsed.is_ok(), "no overflow");
}

/// Unclosed nesting and long runs of clauses or operands, which a
/// quadratic rescan would make slow.
#[test]
fn pathological_queries_parse_in_bounded_time() {
    for (prefix, unit) in [
        ("", "(select "),
        ("", "select 1 union all "),
        ("select * from t ", "join t on "),
        ("", "with x as ("),
        ("select * from ", "("),
        ("select * from ", "(select * from "),
        ("select a from t ", "where a "),
        ("select ", "a, "),
    ] {
        let source = format!("{prefix}{}", unit.repeat((64 << 10) / unit.len()));
        let started = std::time::Instant::now();
        let parsed = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || {
                let parse = crate::syntax::parse(&source, crate::lexer::Mode::Sql);
                parse.syntax().text() == source.as_str()
            })
            .expect("thread starts")
            .join();
        assert!(parsed.is_ok_and(|lossless| lossless), "{unit}");
        assert!(
            started.elapsed().as_secs() < 10,
            "{unit}: {:?}",
            started.elapsed()
        );
    }
}
