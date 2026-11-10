//! `CREATE TABLE` and `CREATE MATERIALIZED VIEW`.
//!
//! The query of `AS SELECT …` and of a projection is the `SELECT` grammar's,
//! so the tree dumps here print it as `<query>` and the inputs keep it in
//! canonical case: the trees and the re-cased texts do not depend on how
//! the query is parsed.

use std::fmt::Write as _;

use rowan::NodeOrToken;

use crate::lexer::Mode;
use crate::syntax::Entry;
use crate::syntax::StatementOutcome;
use crate::syntax::SyntaxKind;
use crate::syntax::SyntaxNode;
use crate::syntax::parse;
use crate::syntax::parse_as;
use crate::syntax::test_support::recased;

/// Whether `node` is a query this grammar hands to the `SELECT` grammar:
/// what follows `AS` other than a table or table function, or what is
/// inside a projection's brackets.
fn is_query(node: &SyntaxNode) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        SyntaxKind::AsClause => !matches!(
            node.kind(),
            SyntaxKind::Name | SyntaxKind::QualifiedName | SyntaxKind::FunctionCall
        ),
        SyntaxKind::Subquery => parent
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::ProjectionDef),
        _ => false,
    }
}

/// `node` without whitespace, one element per line, with queries as
/// `<query>`.
fn render(node: &SyntaxNode, depth: usize, out: &mut String) {
    out.extend(std::iter::repeat_n("  ", depth));
    if is_query(node) {
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

/// The bodies of the statements of `source`, rendered.
fn dump(source: &str) -> String {
    let parse = parse(source, Mode::Jinja);
    assert_eq!(parse.syntax().text().to_string(), source, "lossless");
    let mut out = String::new();
    for statement in parse
        .syntax()
        .children()
        .filter(|n| n.kind() == SyntaxKind::Statement)
    {
        if let Some(body) = statement.first_child() {
            render(&body, 0, &mut out);
        }
    }
    out
}

/// Each input and its tree.
fn trees(inputs: &[&str]) -> String {
    let mut out = String::new();
    for input in inputs {
        let _ = writeln!(out, "> {input}");
        out.push_str(&dump(input));
        out.push('\n');
    }
    out
}

/// Asserts that every statement of `source` parsed without a fallback.
#[track_caller]
fn assert_parses(source: &str) {
    let parse = parse(source, Mode::Jinja);
    assert_eq!(parse.fallbacks(), &[], "{source}");
    for statement in parse.statements() {
        assert_eq!(statement.outcome, StatementOutcome::Parsed, "{source}");
    }
}

#[test]
fn headers() {
    insta::assert_snapshot!(trees(&[
        "CREATE TABLE t (a Int8)",
        "create or replace temporary table if not exists db.t uuid '00000000-0000-0000-0000-000000000001' on cluster '{cluster}' (a Int8)",
        "CREATE TABLE {{ db }}.events_{{ suffix }} ON CLUSTER {{ cluster }} (a Int8)",
        "CREATE TABLE {db:Identifier}.t (a Int8)",
        "CREATE TABLE t TO INNER UUID '00000000-0000-0000-0000-000000000001' ENGINE = SharedSet",
    ]));
}

#[test]
fn column_definitions() {
    insta::assert_snapshot!(trees(&[
        "CREATE TABLE t (a Int8, b Nullable(String) NOT NULL, c Int8 NULL, d NULL DEFAULT 1)",
        "CREATE TABLE t (a Int8 DEFAULT 1, b MATERIALIZED a + 1, c Int8 ALIAS a, d Int8 EPHEMERAL, e Int8 EPHEMERAL 0, f EPHEMERAL now(), g Int8 EPHEMERAL COMMENT 'x', h UInt8 AUTO_INCREMENT, i AUTO_INCREMENT)",
        "CREATE TABLE t (a DateTime CODEC(Delta, ZSTD(3)) TTL a + INTERVAL 1 DAY COMMENT 'x', b Int8 COMMENT 'y' STATISTICS(tdigest, uniq) CODEC(NONE))",
        "CREATE TABLE t (a String COLLATE utf8_unicode_ci NOT NULL, b String NULL COLLATE binary, c Int8 PRIMARY KEY SETTINGS (max_compress_block_size = 1))",
        "CREATE TABLE t (a Int8 DEFAULT 1 NOT NULL, `b c` Array(Tuple(x Int8, y String)), n Nested(k String, v UInt64))",
    ]));
}

#[test]
fn table_elements() {
    insta::assert_snapshot!(trees(&[
        "CREATE TABLE t (a Int8, INDEX i a TYPE minmax GRANULARITY 1, INDEX j (a, lower(s)) TYPE bloom_filter(0.01), INDEX k s TYPE text(tokenizer = 'ngrams'))",
        "CREATE TABLE t (a Int8, PROJECTION p (SELECT a ORDER BY a), PROJECTION q INDEX a, b TYPE basic WITH SETTINGS (index_granularity = 1))",
        "CREATE TABLE t (a Int8, CONSTRAINT c CHECK a > 0, CONSTRAINT d ASSUME a < 10, PRIMARY KEY (a), FOREIGN KEY (a) REFERENCES db.p(b) ON DELETE CASCADE ON UPDATE SET NULL,)",
    ]));
}

#[test]
fn storage_clauses() {
    insta::assert_snapshot!(trees(&[
        "CREATE TABLE t (a Int8) ENGINE = MergeTree() PARTITION BY toYYYYMM(d) PRIMARY KEY a ORDER BY (a, b) SAMPLE BY a SETTINGS index_granularity = 8192, storage_policy = 'x' COMMENT 'c'",
        "CREATE TABLE t (a Int8) ENGINE ReplicatedMergeTree('/p/{shard}', '{replica}') ORDER BY tuple() UNIQUE KEY a",
        "CREATE TABLE t (a Int8) ENGINE = MergeTree ORDER BY a DESC",
        "CREATE TABLE t (a Int8) ENGINE = MergeTree ORDER BY (a DESC, b)",
        "CREATE TABLE t (a Int8) ENGINE = MergeTree ORDER BY tuple(a, b ASC)",
        // Any order, and repeated: query settings after storage settings.
        "CREATE TABLE t (a Int8) SETTINGS a = 1 ORDER BY a ENGINE = Memory SETTINGS b = 2",
    ]));
}

#[test]
fn ttl_rules() {
    insta::assert_snapshot!(trees(&[
        "CREATE TABLE t (d Date) ENGINE = MergeTree ORDER BY d TTL d + INTERVAL 1 MONTH DELETE WHERE x = 1, d + INTERVAL 2 MONTH TO DISK 'a', d + INTERVAL 3 MONTH TO VOLUME IF EXISTS 'b'",
        "CREATE TABLE t (d Date) ENGINE = MergeTree ORDER BY d TTL d RECOMPRESS CODEC(ZSTD(12)), d + 1 GROUP BY k, l SET v = max(v), w = any(w), d + 2 WHERE y",
        // A column called `ttl` in a TTL rule.
        "CREATE TABLE t (ttl Date) ENGINE = MergeTree ORDER BY ttl TTL ttl, ttl + 1 SETTINGS a = 1",
    ]));
}

#[test]
fn as_forms() {
    insta::assert_snapshot!(trees(&[
        "CREATE TABLE t ENGINE = Memory AS SELECT 1",
        "CREATE TABLE t (a Int8) ENGINE = Memory AS SELECT 1",
        "CREATE TABLE t AS db.t2 ENGINE = Memory COMMENT 'x'",
        "CREATE TABLE t AS remote('host', db, t2, SETTINGS connect_timeout = 1, x = 2)",
        "CREATE TABLE t EMPTY AS SELECT 1",
        "CREATE TABLE t CLONE AS t2",
        "CREATE TABLE t (a Int8) ENGINE = MergeTree ORDER BY a DEFINER = default SQL SECURITY DEFINER",
        // Any query may end with `FORMAT`.
        "CREATE TABLE t (a Int8) ENGINE = Memory FORMAT Null",
    ]));
    assert_parses("CREATE TABLE t AS view(SELECT 1)");
}

#[test]
fn materialized_views() {
    insta::assert_snapshot!(trees(&[
        "CREATE MATERIALIZED VIEW IF NOT EXISTS db.v ON CLUSTER c TO db.t AS SELECT 1",
        "CREATE OR REPLACE MATERIALIZED VIEW v UUID '00000000-0000-0000-0000-000000000001' TO t AS SELECT 1",
        "CREATE MATERIALIZED VIEW v TO t (a Int8, b String) AS SELECT 1",
        "CREATE MATERIALIZED VIEW v (a Int8) ENGINE = MergeTree ORDER BY a TTL d TO DISK 'x' POPULATE AS SELECT 1",
        "CREATE MATERIALIZED VIEW v REFRESH EVERY 1 HOUR OFFSET 5 MINUTE RANDOMIZE FOR 1 MINUTE DEPENDS ON a, db.b SETTINGS refresh_retries = 1 APPEND TO t AS SELECT 1",
        "CREATE MATERIALIZED VIEW v REFRESH AFTER 1 HOUR 30 MINUTE ENGINE = Memory EMPTY AS SELECT 1",
        "CREATE MATERIALIZED VIEW v TO t DEFINER = u@'%' SQL SECURITY DEFINER COMMENT 'x' AS SELECT 1",
        "CREATE MATERIALIZED VIEW v TO INNER UUID '00000000-0000-0000-0000-000000000001' ENGINE = Memory AS SELECT 1",
    ]));
}

#[test]
fn time_series_targets() {
    insta::assert_snapshot!(trees(&[
        "CREATE TABLE ts ENGINE = TimeSeries SETTINGS version = 1 DATA db.d TAGS INNER ENGINE = Memory METRICS INNER UUID '00000000-0000-0000-0000-000000000001'",
        "CREATE TABLE ts ENGINE = TimeSeries RECENT SAMPLES INNER COLUMNS (id UUID) TIME RANGES ENGINE = MergeTree ORDER BY id TTL d METRIC FAMILIES m",
    ]));
}

/// A clause that does not parse is kept verbatim up to the next clause, and
/// a column up to the next comma; Jinja block tags and comments between
/// clauses and columns are trivia.
#[test]
fn fallbacks_stay_inside_the_clause_or_column() {
    insta::assert_snapshot!(trees(&[
        "CREATE TABLE t (a Int8, b ?, {% if x %} c Int8 {% endif %}) ENGINE = MergeTree ORDER BY a ?? b TTL d",
        "CREATE TABLE t (a Int8)\n{% if prod %}\nENGINE = ReplicatedMergeTree\n{% else %}\nENGINE = MergeTree\n{% endif %}\n-- the key\nORDER BY a",
        "CREATE TABLE t (index UInt8, `index` UInt8)",
    ]));
}

#[test]
fn the_fixtures_parse_without_fallback() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files = vec![
        root.join("design/page_views_1h.sql.j2"),
        root.join("design/page_views_1h.expected.sql.j2"),
    ];
    for dir in [
        "migrations",
        "template_migrations",
        "bootstrap_migrations",
        "upgrade_migrations",
        "question_mark_migrations",
    ] {
        for entry in std::fs::read_dir(root.join("chum").join(dir)).expect("fixtures") {
            files.push(entry.expect("fixture").path());
        }
    }
    let mut creates = 0;
    for file in files {
        let source = std::fs::read_to_string(&file).expect("fixture");
        let parse = parse(&source, Mode::Jinja);
        for statement in parse.statements() {
            if statement.kind == "create table" || statement.kind == "create materialized view" {
                creates += 1;
                assert_eq!(
                    statement.outcome,
                    StatementOutcome::Parsed,
                    "{}: {}",
                    file.display(),
                    &source[statement.range.clone()]
                );
            }
        }
    }
    assert_eq!(creates, 14);
}

#[test]
fn the_design_example_table() {
    let source = include_str!("../../../../tests/fixtures/design/page_views_1h.sql.j2");
    let table = source.split(';').next().expect("a statement");
    insta::assert_snapshot!(dump(table));
}

/// Statements and their text with every keyword slot in canonical case: the
/// words that are keywords in one place and names in another. The queries
/// are written in canonical case already.
const KEYWORD_SLOTS: &[(&str, &str)] = &[
    (
        "create table t (a Int8) engine = Null",
        "CREATE TABLE t (a Int8) ENGINE = Null",
    ),
    (
        "create table t (a Int8) engine = Null()",
        "CREATE TABLE t (a Int8) ENGINE = Null()",
    ),
    (
        "create table t (a Int8) engine Memory",
        "CREATE TABLE t (a Int8) ENGINE Memory",
    ),
    (
        "create table t (a Int8) engine = Log",
        "CREATE TABLE t (a Int8) ENGINE = Log",
    ),
    (
        "create table t (a Int8) engine = Set",
        "CREATE TABLE t (a Int8) ENGINE = Set",
    ),
    (
        "create table t (a Int8) engine = Join(any, left, a)",
        "CREATE TABLE t (a Int8) ENGINE = Join(any, left, a)",
    ),
    (
        "create table t (a Int8) engine = Dictionary(d)",
        "CREATE TABLE t (a Int8) ENGINE = Dictionary(d)",
    ),
    (
        "create table t (a Int8) engine = Merge(currentDatabase(), '^t')",
        "CREATE TABLE t (a Int8) ENGINE = Merge(currentDatabase(), '^t')",
    ),
    (
        "create table t (a Int8) engine = Overlay(x, y)",
        "CREATE TABLE t (a Int8) ENGINE = Overlay(x, y)",
    ),
    (
        "create table t (primary Int8, foreign Int8) engine = Memory",
        "CREATE TABLE t (primary Int8, foreign Int8) ENGINE = Memory",
    ),
    (
        "create table t (key UInt8, value String, date Date, order UInt8, comment String, ttl UInt8, \
         default UInt8, settings UInt8, engine UInt8, codec UInt8, alias UInt8, type UInt8, \
         partition UInt8, sample UInt8, null UInt8, not UInt8, as UInt8, to UInt8) engine = Null",
        "CREATE TABLE t (key UInt8, value String, date Date, order UInt8, comment String, ttl UInt8, \
         default UInt8, settings UInt8, engine UInt8, codec UInt8, alias UInt8, type UInt8, \
         partition UInt8, sample UInt8, null UInt8, not UInt8, as UInt8, to UInt8) ENGINE = Null",
    ),
    (
        "create table t (`index` UInt8, `projection` UInt8, `constraint` UInt8) engine = Memory",
        "CREATE TABLE t (`index` UInt8, `projection` UInt8, `constraint` UInt8) ENGINE = Memory",
    ),
    (
        "create table t (d Date, a Array(Date), n Nested(x Int8, date Date), i IntervalDay, \
         x Nullable(Int8), y LowCardinality(Nullable(String)), m Map(String, Array(Int8)), \
         e Enum8('a' = 1), j JSON(a.b UInt32, SKIP c), s Nullable(Nothing)) engine = Memory",
        "CREATE TABLE t (d Date, a Array(Date), n Nested(x Int8, date Date), i IntervalDay, \
         x Nullable(Int8), y LowCardinality(Nullable(String)), m Map(String, Array(Int8)), \
         e Enum8('a' = 1), j JSON(a.b UInt32, SKIP c), s Nullable(Nothing)) ENGINE = Memory",
    ),
    (
        "create table t (a Int8 codec(Delta, ZSTD(3)), b Int8 codec(NONE), c Int8 codec(Default), \
         d Int8 codec(lz4, delta(4))) engine = Memory",
        "CREATE TABLE t (a Int8 CODEC(Delta, ZSTD(3)), b Int8 CODEC(NONE), c Int8 CODEC(Default), \
         d Int8 CODEC(lz4, delta(4))) ENGINE = Memory",
    ),
    (
        "create table t (default UInt8 default 1, x UInt8 default default, y Int8 ephemeral codec(ZSTD)) \
         engine = Memory",
        "CREATE TABLE t (default UInt8 DEFAULT 1, x UInt8 DEFAULT default, y Int8 EPHEMERAL codec(ZSTD)) \
         ENGINE = Memory",
    ),
    (
        "create table t (a Int8 not null, b Int8 null, c null default 1, d Int8 ephemeral, \
         e Int8 auto_increment, f String collate utf8_bin, g Int8 materialized 1, h Int8 alias g, \
         i Int8 ttl d + interval 1 day, j Int8 comment 'x', k Int8 primary key, \
         l Int8 statistics(tdigest, uniq), m Int8 settings (max_compress_block_size = 1)) engine = Memory",
        "CREATE TABLE t (a Int8 NOT NULL, b Int8 NULL, c NULL DEFAULT 1, d Int8 EPHEMERAL, \
         e Int8 AUTO_INCREMENT, f String COLLATE utf8_bin, g Int8 MATERIALIZED 1, h Int8 ALIAS g, \
         i Int8 TTL d + INTERVAL 1 DAY, j Int8 COMMENT 'x', k Int8 PRIMARY KEY, \
         l Int8 STATISTICS(tdigest, uniq), m Int8 SETTINGS (max_compress_block_size = 1)) ENGINE = Memory",
    ),
    (
        "create table t (comment String comment 'c', a Int8) engine = Memory comment 'x'",
        "CREATE TABLE t (comment String COMMENT 'c', a Int8) ENGINE = Memory COMMENT 'x'",
    ),
    (
        "create table t (a Int8) engine = MergeTree partition by d comment 'x'",
        "CREATE TABLE t (a Int8) ENGINE = MergeTree PARTITION BY d COMMENT 'x'",
    ),
    (
        "create table t (a Int8) engine = Distributed(c, db, settings) settings a = 1",
        "CREATE TABLE t (a Int8) ENGINE = Distributed(c, db, settings) SETTINGS a = 1",
    ),
    (
        "create table t (settings UInt8 settings (max_compress_block_size = 1)) engine = MergeTree \
         order by tuple()",
        "CREATE TABLE t (settings UInt8 SETTINGS (max_compress_block_size = 1)) ENGINE = MergeTree \
         ORDER BY tuple()",
    ),
    (
        "create table t (a Int8, index i a type set(100) granularity 2, index j a type minmax, \
         projection p (SELECT a ORDER BY a), projection q index a type basic \
         with settings (index_granularity = 1), constraint c check a > 0, constraint d assume a < 1, \
         primary key a, foreign key (a) references p(a) on delete cascade on update set null) \
         engine = MergeTree order by a",
        "CREATE TABLE t (a Int8, INDEX i a TYPE set(100) GRANULARITY 2, INDEX j a TYPE minmax, \
         PROJECTION p (SELECT a ORDER BY a), PROJECTION q INDEX a TYPE basic \
         WITH SETTINGS (index_granularity = 1), CONSTRAINT c CHECK a > 0, CONSTRAINT d ASSUME a < 1, \
         PRIMARY KEY a, FOREIGN KEY (a) REFERENCES p(a) ON DELETE CASCADE ON UPDATE SET NULL) \
         ENGINE = MergeTree ORDER BY a",
    ),
    (
        "create table t (a Int8) engine = MergeTree partition by toYYYYMM(d) primary key a \
         order by (a, b) sample by a unique key a ttl d + interval 1 day settings index_granularity = 1",
        "CREATE TABLE t (a Int8) ENGINE = MergeTree PARTITION BY toYYYYMM(d) PRIMARY KEY a \
         ORDER BY (a, b) SAMPLE BY a UNIQUE KEY a TTL d + INTERVAL 1 DAY SETTINGS index_granularity = 1",
    ),
    (
        "create table t (a Int8) engine = MergeTree order by tuple(a desc, b)",
        "CREATE TABLE t (a Int8) ENGINE = MergeTree ORDER BY tuple(a DESC, b)",
    ),
    (
        "create table t (a Int8) engine = MergeTree order by (a desc, b asc)",
        "CREATE TABLE t (a Int8) ENGINE = MergeTree ORDER BY (a DESC, b ASC)",
    ),
    (
        "create table t (a Int8) engine = MergeTree order by a desc",
        "CREATE TABLE t (a Int8) ENGINE = MergeTree ORDER BY a DESC",
    ),
    (
        "create table t (d Date) engine = MergeTree order by d ttl d + interval 1 day delete where x = 1, \
         d + interval 2 day to disk 'a', d to volume if exists 'b', d recompress codec(ZSTD(12)), \
         d group by k set v = max(v)",
        "CREATE TABLE t (d Date) ENGINE = MergeTree ORDER BY d TTL d + INTERVAL 1 DAY DELETE WHERE x = 1, \
         d + INTERVAL 2 DAY TO DISK 'a', d TO VOLUME IF EXISTS 'b', d RECOMPRESS CODEC(ZSTD(12)), \
         d GROUP BY k SET v = max(v)",
    ),
    (
        "create table t (ttl Date) engine = MergeTree order by ttl ttl ttl, ttl + 1",
        "CREATE TABLE t (ttl Date) ENGINE = MergeTree ORDER BY ttl TTL ttl, ttl + 1",
    ),
    (
        "create table {{ db }}.t_{{ s }} (a Int8) engine = {{ engine }} settings a = {{ a }}",
        "CREATE TABLE {{ db }}.t_{{ s }} (a Int8) ENGINE = {{ engine }} SETTINGS a = {{ a }}",
    ),
    (
        "create table t (a Int8) engine = Memory format Null",
        "CREATE TABLE t (a Int8) ENGINE = Memory FORMAT Null",
    ),
    (
        "create table t (a Int8) engine = MergeTree order by a definer = default sql security definer",
        "CREATE TABLE t (a Int8) ENGINE = MergeTree ORDER BY a DEFINER = default SQL SECURITY DEFINER",
    ),
    (
        "create table t engine = Memory empty as SELECT 1",
        "CREATE TABLE t ENGINE = Memory EMPTY AS SELECT 1",
    ),
    ("create table t clone as t2", "CREATE TABLE t CLONE AS t2"),
    (
        "create table t as db.t2 engine = Memory",
        "CREATE TABLE t AS db.t2 ENGINE = Memory",
    ),
    (
        "create table t as remote('a', db, t, settings x = 1)",
        "CREATE TABLE t AS remote('a', db, t, SETTINGS x = 1)",
    ),
    (
        "create materialized view v to t as SELECT 1",
        "CREATE MATERIALIZED VIEW v TO t AS SELECT 1",
    ),
    (
        "create or replace materialized view v on cluster c to db.t (a Int8) as SELECT 1",
        "CREATE OR REPLACE MATERIALIZED VIEW v ON CLUSTER c TO db.t (a Int8) AS SELECT 1",
    ),
    (
        "create materialized view if not exists v to t as SELECT 1",
        "CREATE MATERIALIZED VIEW IF NOT EXISTS v TO t AS SELECT 1",
    ),
    (
        "create materialized view v engine = MergeTree order by d ttl d to disk 'x' populate as SELECT 1",
        "CREATE MATERIALIZED VIEW v ENGINE = MergeTree ORDER BY d TTL d TO DISK 'x' POPULATE AS SELECT 1",
    ),
    (
        "create materialized view v refresh every 1 hour offset 5 minute randomize for 1 minute \
         depends on a, db.b settings refresh_retries = 1 append to t as SELECT 1",
        "CREATE MATERIALIZED VIEW v REFRESH EVERY 1 HOUR OFFSET 5 MINUTE RANDOMIZE FOR 1 MINUTE \
         DEPENDS ON a, db.b SETTINGS refresh_retries = 1 APPEND TO t AS SELECT 1",
    ),
    (
        "create materialized view v to t comment 'x' as SELECT 1",
        "CREATE MATERIALIZED VIEW v TO t COMMENT 'x' AS SELECT 1",
    ),
    (
        "create table ts engine = TimeSeries tags inner engine = Memory samples db.s \
         recent samples inner uuid '00000000-0000-0000-0000-000000000001'",
        "CREATE TABLE ts ENGINE = TimeSeries TAGS INNER ENGINE = Memory SAMPLES db.s \
         RECENT SAMPLES INNER UUID '00000000-0000-0000-0000-000000000001'",
    ),
    (
        "create table ts engine = TimeSeries data inner columns (id UUID) \
         metric families inner engine = MergeTree order by id",
        "CREATE TABLE ts ENGINE = TimeSeries DATA INNER COLUMNS (id UUID) \
         METRIC FAMILIES INNER ENGINE = MergeTree ORDER BY id",
    ),
];

#[test]
fn keyword_slots() {
    for (input, expected) in KEYWORD_SLOTS {
        assert_parses(input);
        assert_eq!(&recased(input, Entry::File), expected, "{input}");
        // The verifier lets each recorded word change only into its
        // canonical case, so a slot on a word that is no keyword fails here.
        let parse = parse_as(input, Mode::Jinja, Entry::File);
        crate::verify::verify(input, expected, Mode::Jinja, parse.facts())
            .unwrap_or_else(|e| panic!("{input}: {e}"));
    }
}

/// The keyword slots against ClickHouse: `clickhouse-format` must print the
/// re-cased statement as it prints the input, which it does not if a name
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
    let texts: Vec<&str> = KEYWORD_SLOTS
        .iter()
        .filter(|(input, _)| !input.contains("{{"))
        .flat_map(|(input, expected)| [*input, *expected])
        .collect();
    let verdicts = oracle.canonical(&texts);
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

/// ClickHouse rejects a column called `index`, `projection` or `constraint`
/// unless it is quoted, since the word starts that element.
#[test]
fn element_keywords_are_not_column_names() {
    for word in ["index", "projection", "constraint"] {
        let source = format!("CREATE TABLE t ({word} UInt8, b UInt8) ENGINE = Memory");
        let parse = parse(&source, Mode::Sql);
        let [fallback] = parse.fallbacks() else {
            panic!("{source}: one fallback");
        };
        assert_eq!(&source[fallback.range.clone()], format!("{word} UInt8"));
    }
}

#[test]
fn clause_keywords_inside_expressions_are_names() {
    // `empty(s)` is a function, `engine = 1` and `ttl + 1` expressions.
    assert_parses(
        "CREATE TABLE t (s String, ttl Date) ENGINE = MergeTree ORDER BY s \
         TTL ttl + 1, empty(s) + ttl SETTINGS a = 1",
    );
    assert_parses("CREATE TABLE t (s String) ENGINE = Memory SETTINGS a = 1, b = 2 COMMENT 'x'");
}

#[test]
fn deep_nesting_in_a_column_becomes_verbatim_without_overflowing() {
    let deep = 20_000;
    for source in [
        format!(
            "CREATE TABLE t (a {}Int8{})",
            "Array(".repeat(deep),
            ")".repeat(deep)
        ),
        format!(
            "CREATE TABLE t (a Int8 DEFAULT {}1{})",
            "(".repeat(deep),
            ")".repeat(deep)
        ),
    ] {
        let parse = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || {
                let parse = parse(&source, Mode::Sql);
                assert_eq!(parse.syntax().text().to_string(), source);
                parse.fallbacks().len()
            })
            .expect("thread starts")
            .join()
            .expect("no overflow");
        assert!(parse > 0);
    }
}

#[test]
fn many_columns_parse_in_linear_time() {
    let columns: Vec<String> = (0..5000)
        .map(|i| format!("c{i} Int8 DEFAULT {i} CODEC(ZSTD(1)) COMMENT 'x'"))
        .collect();
    let source = format!(
        "CREATE TABLE t ({}) ENGINE = MergeTree ORDER BY (c0 DESC, c1) TTL {}",
        columns.join(", "),
        vec!["c0 + 1"; 2000].join(", ")
    );
    let started = std::time::Instant::now();
    let parse = parse(&source, Mode::Sql);
    assert_eq!(parse.fallbacks(), &[]);
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
}
