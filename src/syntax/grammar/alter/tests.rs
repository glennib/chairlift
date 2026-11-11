//! `ALTER` and the lightweight mutations.

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
fn column_commands() {
    insta::assert_snapshot!(trees(&[
        "ALTER TABLE db.t ON CLUSTER c ADD COLUMN IF NOT EXISTS n.x Nullable(Int8) DEFAULT 1 CODEC(ZSTD) AFTER y",
        "ALTER TABLE t ADD COLUMN a Int8 FIRST, DROP COLUMN IF EXISTS b, RENAME COLUMN c TO d",
        "ALTER TABLE t MODIFY COLUMN a UInt64 DEFAULT 0 COMMENT 'x' AFTER b",
        "ALTER TABLE t MODIFY COLUMN a REMOVE TTL, MODIFY COLUMN b COMMENT 'c', MODIFY COLUMN c CODEC(LZ4)",
        "ALTER TABLE t MODIFY COLUMN a MODIFY SETTING max_compress_block_size = 1, b = 2",
        "ALTER TABLE t MODIFY COLUMN IF EXISTS a RESET SETTING x, y",
        "ALTER TABLE t ALTER COLUMN a TYPE Int16, CLEAR COLUMN b IN PARTITION 1",
        "ALTER TABLE t COMMENT COLUMN IF EXISTS a 'text', MATERIALIZE COLUMN b IN PARTITION ID '1'",
    ]));
}

#[test]
fn table_commands() {
    insta::assert_snapshot!(trees(&[
        "ALTER TABLE t MODIFY ORDER BY (a, b), MODIFY SAMPLE BY c, REMOVE SAMPLE BY",
        "ALTER TABLE t MODIFY TTL d + INTERVAL 1 DAY DELETE, d + INTERVAL 1 HOUR TO VOLUME 'v'",
        "ALTER TABLE t REMOVE TTL, MATERIALIZE TTL IN PARTITION 2, REWRITE PARTS",
        "ALTER TABLE t MODIFY SETTING a = 1, b = 'x'",
        "ALTER TABLE t RESET SETTING a, b",
        "ALTER TABLE t MODIFY COMMENT 'x', APPLY DELETED MASK IN PARTITION 1, APPLY PATCHES",
        "ALTER TABLE mv MODIFY QUERY SELECT a FROM t WHERE b",
        "ALTER TABLE mv MODIFY REFRESH EVERY 1 DAY, MODIFY SQL SECURITY INVOKER",
        "ALTER DATABASE d ON CLUSTER c MODIFY SETTING a = 1",
        "ALTER DATABASE d MODIFY COMMENT 'x'",
        "ALTER TEMPORARY TABLE t DROP COLUMN a",
        "ALTER TABLE t (ADD COLUMN a Int8), (DROP COLUMN b)",
    ]));
}

#[test]
fn index_projection_constraint_statistics() {
    insta::assert_snapshot!(trees(&[
        "ALTER TABLE t ADD INDEX IF NOT EXISTS i a TYPE minmax GRANULARITY 1 AFTER j",
        "ALTER TABLE t DROP INDEX i, CLEAR INDEX IF EXISTS j IN PARTITION 1, MATERIALIZE INDEX k",
        "ALTER TABLE t ADD PROJECTION p (SELECT a ORDER BY b) FIRST, MATERIALIZE PROJECTION p",
        "ALTER TABLE t MODIFY PROJECTION p (SELECT a), DROP PROJECTION IF EXISTS q, CLEAR PROJECTION r",
        "ALTER TABLE t ADD CONSTRAINT c CHECK a > 1, DROP CONSTRAINT IF EXISTS d",
        "ALTER TABLE t ADD STATISTICS IF NOT EXISTS a, b TYPE tdigest, uniq",
        "ALTER TABLE t MODIFY STATISTICS a TYPE countmin",
        "ALTER TABLE t DROP STATISTICS a, b",
        "ALTER TABLE t CLEAR STATISTICS ALL",
        "ALTER TABLE t MATERIALIZE STATISTICS IF EXISTS a IN PARTITION 1",
    ]));
}

#[test]
fn mutations_and_partitions() {
    insta::assert_snapshot!(trees(&[
        "ALTER TABLE t DELETE WHERE a = 1 AND b SETTINGS mutations_sync = 2",
        "ALTER TABLE t DELETE IN PARTITION 1, ID '2' WHERE a",
        "ALTER TABLE t UPDATE a = a + 1, b = 'x' IN PARTITION tuple() WHERE c",
        "ALTER TABLE t DROP PARTITION 202601, DROP PART 'all_1_1_0', DROP DETACHED PARTITION ALL",
        "ALTER TABLE t DETACH PARTITION ID '1', ATTACH PARTITION 1 FROM db.u, ATTACH PART 'p' FROM 'x'",
        "ALTER TABLE t REPLACE PARTITION 1 FROM u, MOVE PARTITION 2 TO TABLE v, MOVE PART 'p' TO DISK 'd'",
        "ALTER TABLE t FETCH PARTITION 1 FROM '/clickhouse/t', FORGET PARTITION 2",
        "ALTER TABLE t FREEZE PARTITION 1 WITH NAME 'b', UNFREEZE WITH NAME 'b', FREEZE",
        "ALTER TABLE t EXECUTE expire_snapshots('2026-01-01')",
        "DELETE FROM db.t ON CLUSTER c IN PARTITION 1 WHERE a = 1 SETTINGS lightweight_deletes_sync = 0",
        "UPDATE t SET a = 1, b = b + 1 IN PARTITION 2 WHERE c",
    ]));
}

/// Names that are keyword words stay names, and keywords where ClickHouse
/// matches them; the re-cased text passes the verifier.
#[test]
fn keyword_slots() {
    for (source, expected) in [
        (
            "alter table t drop column index",
            "ALTER TABLE t DROP COLUMN index",
        ),
        (
            "alter table t rename column to to from",
            "ALTER TABLE t RENAME COLUMN to TO from",
        ),
        (
            "alter table t add column first Int8 first",
            "ALTER TABLE t ADD COLUMN first Int8 FIRST",
        ),
        (
            "alter table t add column a Int8 after after",
            "ALTER TABLE t ADD COLUMN a Int8 AFTER after",
        ),
        (
            "alter table t drop index index",
            "ALTER TABLE t DROP INDEX index",
        ),
        (
            "alter table t add index type type type minmax",
            "ALTER TABLE t ADD INDEX type type TYPE minmax",
        ),
        (
            "alter table t drop projection select",
            "ALTER TABLE t DROP PROJECTION select",
        ),
        (
            "alter table t drop constraint check",
            "ALTER TABLE t DROP CONSTRAINT check",
        ),
        (
            "alter table t modify setting setting = 1",
            "ALTER TABLE t MODIFY SETTING setting = 1",
        ),
        (
            "alter table t reset setting reset",
            "ALTER TABLE t RESET SETTING reset",
        ),
        (
            "alter table t drop statistics type",
            "ALTER TABLE t DROP STATISTICS type",
        ),
        (
            "alter table t drop partition id '1'",
            "ALTER TABLE t DROP PARTITION ID '1'",
        ),
        (
            "alter table t drop partition all",
            "ALTER TABLE t DROP PARTITION ALL",
        ),
        (
            "alter table t drop partition partition",
            "ALTER TABLE t DROP PARTITION partition",
        ),
        (
            "alter table t modify column remove remove ttl",
            "ALTER TABLE t MODIFY COLUMN remove REMOVE TTL",
        ),
        (
            "alter table t update set = 1 where where",
            "ALTER TABLE t UPDATE set = 1 WHERE where",
        ),
        (
            "alter table table delete where 1",
            "ALTER TABLE table DELETE WHERE 1",
        ),
        ("delete from from where 1", "DELETE FROM from WHERE 1"),
        (
            "update set set set = 1 where 1",
            "UPDATE set SET set = 1 WHERE 1",
        ),
        (
            "alter database database modify comment 'x'",
            "ALTER DATABASE database MODIFY COMMENT 'x'",
        ),
    ] {
        let out = recased(source, Entry::File);
        assert_eq!(out, expected, "{source}");
        let parse = parse(source, Mode::Sql);
        verify(source, &out, Mode::Sql, parse.facts()).unwrap_or_else(|e| panic!("{source}: {e}"));
    }
}

/// What ClickHouse rejects fails to parse here too, and access entities
/// are left to their own grammar.
#[test]
fn what_is_not_parsed_stays_verbatim() {
    for source in [
        "ALTER TABLE t",
        "ALTER TABLE t ADD COLUMN a",
        "ALTER TABLE t RENAME COLUMN a b",
        "ALTER TABLE t DELETE a = 1",
        "ALTER TABLE t UNFREEZE",
        "ALTER TABLE t MOVE PART 'p' TO TABLE u",
        "DELETE FROM t",
        "UPDATE t SET a = 1",
    ] {
        assert_ne!(outcome(source), StatementOutcome::Parsed, "{source}");
    }
}
