//! Statements of a line or two.

use std::fmt::Write as _;

use crate::lexer::Mode;
use crate::syntax::Entry;
use crate::syntax::StatementOutcome;
use crate::syntax::parse;
use crate::syntax::test_support::recased;
use crate::syntax::test_support::tree;

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
fn drop_detach_truncate_undrop() {
    insta::assert_snapshot!(trees(&[
        "DROP TABLE IF EXISTS db.a, b ON CLUSTER '{cluster}' SYNC",
        "drop temporary table if empty t no delay",
        "DROP VIEW v",
        "DROP DICTIONARY IF EXISTS {{ db }}.d",
        "DROP DATABASE IF EXISTS {db:Identifier} ON CLUSTER c",
        "DROP FUNCTION IF EXISTS f ON CLUSTER c",
        "DROP NAMED COLLECTION IF EXISTS nc",
        "DROP INDEX IF EXISTS i ON db.t",
        "DROP WORKLOAD w",
        "DROP HANDLER IF EXISTS h",
        "DETACH TABLE t ON CLUSTER c PERMANENTLY SYNC",
        "DETACH DATABASE d",
        "TRUNCATE t",
        "TRUNCATE TABLE IF EXISTS db.t ON CLUSTER c",
        "TRUNCATE ALL TABLES FROM IF EXISTS db NOT LIKE 'tmp%'",
        "UNDROP TABLE db.t UUID '00000000-0000-0000-0000-000000000001' ON CLUSTER c",
        "DROP TABLE t FORMAT Null SETTINGS max_execution_time = 1",
    ]));
}

#[test]
fn attach() {
    insta::assert_snapshot!(trees(&[
        "ATTACH TABLE IF NOT EXISTS db.t ON CLUSTER c",
        "ATTACH TABLE t UUID '00000000-0000-0000-0000-000000000001' AS NOT REPLICATED",
        "ATTACH DICTIONARY d",
        "ATTACH DATABASE {db:Identifier}",
        "ATTACH MATERIALIZED VIEW mv",
        // A definition after the name is kept as written.
        "ATTACH TABLE t (a UInt8) ENGINE = Memory",
        "ATTACH TABLE t FROM '/data/t' (a UInt8) ENGINE = MergeTree ORDER BY a",
        "ATTACH DATABASE d ENGINE = Atomic",
    ]));
}

#[test]
fn rename_and_exchange() {
    insta::assert_snapshot!(trees(&[
        "RENAME TABLE IF EXISTS db.a TO db.b, c TO d ON CLUSTER c",
        "RENAME a TO b",
        "RENAME DICTIONARY d TO e",
        "RENAME DATABASE IF EXISTS a TO b ON CLUSTER c",
        "EXCHANGE TABLES a AND db.b",
        "EXCHANGE DICTIONARIES a AND b, c AND d",
    ]));
}

#[test]
fn optimize() {
    insta::assert_snapshot!(trees(&[
        "OPTIMIZE TABLE db.t ON CLUSTER c PARTITION tuple() FINAL DEDUPLICATE BY a, COLUMNS('b') EXCEPT c, * EXCEPT (d)",
        "OPTIMIZE TABLE t PARTITION ID '202601' FORCE CLEANUP",
        "OPTIMIZE TABLE t PARTITION ALL",
        "OPTIMIZE TABLE t PARTITION (2026, 'a') DRY RUN PARTS 'all_1_1_0', 'all_2_2_0'",
        "OPTIMIZE TABLE t PARTITION ID {p:String} SETTINGS optimize_throw_if_noop = 1",
    ]));
}

#[test]
fn set_and_use() {
    insta::assert_snapshot!(trees(&[
        "SET max_threads = 1, send_logs_level = 'error', max_block_size = DEFAULT, enable_analyzer",
        "SET param_x = [1, 2], additional_table_filters = {'t': 'x > 1'}",
        "SET TIME ZONE 'Europe/Oslo'",
        "SET time_zone = 'UTC'",
        "USE system",
        "USE DATABASE db",
        "USE database",
        "USE {{ db }}",
    ]));
}

/// Words that are keywords somewhere are names where a name stands, and
/// keywords where ClickHouse matches them.
#[test]
fn keyword_slots() {
    for (source, expected) in [
        ("drop table sync sync", "DROP TABLE sync SYNC"),
        ("drop table if", "DROP TABLE if"),
        (
            "drop table on, if on cluster c",
            "DROP TABLE on, if ON CLUSTER c",
        ),
        ("drop database default", "DROP DATABASE default"),
        ("rename table a to to", "RENAME TABLE a TO to"),
        ("exchange tables and and to", "EXCHANGE TABLES and AND to"),
        ("optimize table final final", "OPTIMIZE TABLE final FINAL"),
        (
            "optimize table t deduplicate by by",
            "OPTIMIZE TABLE t DEDUPLICATE BY by",
        ),
        ("use system", "USE system"),
        ("use database", "USE database"),
        ("use database database", "USE DATABASE database"),
        ("set format = 'x'", "SET format = 'x'"),
        ("set role = 1", "SET role = 1"),
        ("set default = 1", "SET default = 1"),
        ("set x = default", "SET x = DEFAULT"),
        ("set time zone = 'UTC'", "SET TIME ZONE = 'UTC'"),
        ("attach table table", "ATTACH TABLE table"),
        ("undrop table uuid", "UNDROP TABLE uuid"),
    ] {
        assert_eq!(recased(source, Entry::File), expected, "{source}");
    }
}

/// What these statements' grammars do not cover stays verbatim as a whole;
/// what ClickHouse rejects too fails to parse.
#[test]
fn what_is_not_parsed_stays_verbatim() {
    for source in [
        "DROP USER u",
        "DROP ROW POLICY p ON t",
        "SET ROLE r",
        "SET DEFAULT ROLE r TO u",
        "SET TRANSACTION SNAPSHOT 1",
        "ATTACH PARTITION 1",
        "OPTIMIZE t",
        "DETACH TABLE a, b",
        "TRUNCATE table",
        "DROP TABLE t ON c",
        "RENAME DATABASE a TO b, c TO d",
        "OPTIMIZE TABLE t PARTITION id",
        "USE",
    ] {
        assert_eq!(outcome(source), StatementOutcome::Verbatim, "{source}");
    }
}
