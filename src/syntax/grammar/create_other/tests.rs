//! `CREATE DICTIONARY`, `CREATE FUNCTION`, and the views and databases of
//! `create.rs`.

use std::fmt::Write as _;

use crate::lexer::Mode;
use crate::syntax::Entry;
use crate::syntax::StatementOutcome;
use crate::syntax::parse;
use crate::syntax::test_support::recased;
use crate::syntax::test_support::tree;
use crate::verify::verify;

/// Each input and its tree.
fn trees(inputs: &[&str]) -> String {
    let mut out = String::new();
    for input in inputs {
        let _ = writeln!(out, "> {input}");
        out.push_str(&tree(input, Entry::File));
        out.push('\n');
    }
    out
}

/// The outcome of the first statement of `source`.
fn outcome(source: &str) -> StatementOutcome {
    parse(source, Mode::Jinja).statements()[0].outcome
}

#[test]
fn dictionaries() {
    insta::assert_snapshot!(trees(&[
        "CREATE DICTIONARY IF NOT EXISTS db.d ON CLUSTER c (id UInt64, name String DEFAULT '' EXPRESSION lower(n) HIERARCHICAL INJECTIVE, parent UInt64 DEFAULT 0 IS_OBJECT_ID,) PRIMARY KEY id SOURCE(CLICKHOUSE(HOST 'localhost' PORT 9000 USER 'default' PASSWORD 'secret' DB 'db' TABLE 't')) LAYOUT(HASHED()) LIFETIME(MIN 0 MAX 1000) COMMENT 'x'",
        "CREATE OR REPLACE DICTIONARY d (k String, a Array(UInt8) DEFAULT [1, 2]) PRIMARY KEY (k) SOURCE(clickhouse(table 'x' query 'SELECT 1')) LAYOUT(COMPLEX_KEY_HASHED(SHARDS 4)) LIFETIME(300) RANGE(MIN start MAX end) SETTINGS(format_csv_allow_single_quotes = 0)",
        "CREATE DICTIONARY d (id UInt64) PRIMARY KEY id LAYOUT(FLAT) SOURCE(MONGODB(OPTIONS (a 1) URI 'x')) LIFETIME(MIN 0 MAX 0)",
    ]));
}

#[test]
fn functions_views_and_databases() {
    insta::assert_snapshot!(trees(&[
        "CREATE FUNCTION f AS (x, y) -> x + y",
        "CREATE OR REPLACE FUNCTION IF NOT EXISTS f ON CLUSTER c AS x -> if(x > 0, x, 0)",
        "CREATE FUNCTION f LANGUAGE WASM ARGUMENTS (a Int32) RETURNS Int32 FROM 'm'",
        "CREATE VIEW IF NOT EXISTS db.v ON CLUSTER c (a UInt8) AS SELECT 1 AS a COMMENT 'x'",
        "CREATE OR REPLACE VIEW v DEFINER = u SQL SECURITY DEFINER AS SELECT * FROM t WHERE x = {p:UInt8}",
        "CREATE TEMPORARY VIEW v AS (SELECT 1)",
        "CREATE DATABASE IF NOT EXISTS d ON CLUSTER c ENGINE = Replicated('/x', '{shard}', '{replica}') SETTINGS a = 1 COMMENT 'x'",
        "CREATE DATABASE d",
    ]));
}

#[test]
fn indexes() {
    insta::assert_snapshot!(trees(&[
        "CREATE INDEX i ON t (a DESC, b)",
        "CREATE UNIQUE INDEX IF NOT EXISTS i ON db.t ON CLUSTER c a + 1 TYPE bloom_filter(0.01) GRANULARITY 4",
    ]));
}

/// Inside `SOURCE`, `LAYOUT`, `LIFETIME` and `RANGE` every word is a name;
/// a function's name and parameters are names; the re-cased text passes the
/// verifier.
#[test]
fn keyword_slots() {
    for (source, expected) in [
        (
            "create dictionary d (id UInt64) primary key id source(clickhouse(table 'x' db 'y' user 'u' password 'p' where 'a' query 'q')) layout(flat()) lifetime(min 0 max 1)",
            "CREATE DICTIONARY d (id UInt64) PRIMARY KEY id SOURCE(clickhouse(table 'x' db 'y' user 'u' password 'p' where 'a' query 'q')) LAYOUT(flat()) LIFETIME(min 0 max 1)",
        ),
        (
            "create dictionary d (key UInt64 default 0 hierarchical, source String expression lower(layout)) primary key key source(file(path 'x' format 'TSV')) layout(range_hashed()) range(min from max to) lifetime(0)",
            "CREATE DICTIONARY d (key UInt64 DEFAULT 0 HIERARCHICAL, source String EXPRESSION lower(layout)) PRIMARY KEY key SOURCE(file(path 'x' format 'TSV')) LAYOUT(range_hashed()) RANGE(min from max to) LIFETIME(0)",
        ),
        (
            "create function select as (from, to) -> from + to",
            "CREATE FUNCTION select AS (from, to) -> from + to",
        ),
        (
            "create view view as select 1",
            "CREATE VIEW view AS SELECT 1",
        ),
        (
            "create database database engine = Atomic",
            "CREATE DATABASE database ENGINE = Atomic",
        ),
        (
            "create index index on table (type desc) type set(1) granularity 1",
            "CREATE INDEX index ON table (type DESC) TYPE set(1) GRANULARITY 1",
        ),
    ] {
        let out = recased(source, Entry::File);
        assert_eq!(out, expected, "{source}");
        let parse = parse(source, Mode::Sql);
        verify(source, &out, Mode::Sql, parse.facts()).unwrap_or_else(|e| panic!("{source}: {e}"));
    }
}

#[test]
fn what_is_not_parsed_stays_verbatim() {
    for source in [
        "CREATE DICTIONARY d PRIMARY KEY id",
        "CREATE DICTIONARY d (id) PRIMARY KEY id",
        "CREATE FUNCTION f",
        "CREATE INDEX i ON t () TYPE minmax",
        "CREATE INDEX i ON t (a) TYPE minmax GRANULARITY x",
    ] {
        assert_ne!(outcome(source), StatementOutcome::Parsed, "{source}");
    }
}
