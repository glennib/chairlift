use crate::Options;

/// Formats `source` at `width`, checking that formatting again changes
/// nothing.
fn fmt_in(source: &str, width: u16) -> String {
    let options = Options::default().with_line_width(width);
    let once = crate::format(source, &options).unwrap_or_else(|e| panic!("{source:?}: {e}"));
    let twice = crate::format(&once, &options).unwrap_or_else(|e| panic!("{once:?}: {e}"));
    assert_eq!(twice, once, "not idempotent on {source:?}");
    once
}

/// Checks that `source` formats to `expected` at `width`, and `expected`
/// to itself.
fn check_at(source: &str, expected: &str, width: u16) {
    let expected = expected.strip_prefix('\n').unwrap_or(expected);
    assert_eq!(fmt_in(source, width), expected, "from {source:?}");
    assert_eq!(fmt_in(expected, width), expected);
}

fn check(source: &str, expected: &str) {
    check_at(source, expected, 100);
}

#[test]
fn one_command_stays_on_the_alter_line() {
    check(
        "alter table t drop column if exists a;\nalter table db.t on cluster c\n  modify setting a = 1, b = 2;\n\
         alter table t delete where x = 1 settings mutations_sync = 2;",
        "
ALTER TABLE t DROP COLUMN IF EXISTS a;
ALTER TABLE db.t ON CLUSTER c MODIFY SETTING a = 1, b = 2;
ALTER TABLE t DELETE WHERE x = 1 SETTINGS mutations_sync = 2;
",
    );
}

#[test]
fn several_commands_go_one_per_line() {
    check(
        "alter table t add column a Int8 first, drop column b, rename column c to d",
        "
ALTER TABLE t
    ADD COLUMN a Int8 FIRST,
    DROP COLUMN b,
    RENAME COLUMN c TO d
",
    );
}

/// A command that does not fit goes on the next line, and breaks inside
/// before `WHERE`; its list breaks one item per line.
#[test]
fn a_long_command_breaks_inside() {
    check_at(
        "alter table events update status = 'archived', archived_at = now() where event_time < now() - interval 1 year",
        "
ALTER TABLE events
    UPDATE status = 'archived', archived_at = now()
        WHERE event_time < now() - INTERVAL 1 YEAR
",
        60,
    );
    check_at(
        "alter table events update status = 'archived', archived_at = now(), reason = 'too old' where 1",
        "
ALTER TABLE events
    UPDATE status = 'archived',
        archived_at = now(),
        reason = 'too old'
        WHERE 1
",
        40,
    );
}

/// A column definition breaks before its modifiers as in `CREATE TABLE`.
#[test]
fn column_definitions_break_as_in_create_table() {
    check_at(
        "alter table t modify column visitor_id String default '' codec(ZSTD(1)) comment 'Anonymous visitor' after site",
        "
ALTER TABLE t
    MODIFY COLUMN visitor_id String
        DEFAULT ''
        CODEC(ZSTD(1))
        COMMENT 'Anonymous visitor' AFTER site
",
        50,
    );
}

#[test]
fn modify_query_puts_the_query_on_its_lines() {
    check(
        "alter table mv modify query select a, b from t where c = 1",
        "
ALTER TABLE mv MODIFY QUERY
SELECT
    a,
    b
FROM t
WHERE c = 1
",
    );
    check(
        "alter table mv modify comment 'x', modify query select a from t",
        "
ALTER TABLE mv
    MODIFY COMMENT 'x',
    MODIFY QUERY
    SELECT a
    FROM t
",
    );
}

#[test]
fn partition_commands_stay_on_one_line() {
    check(
        "alter table t drop partition id '2026', move partition 2 to table u, freeze partition 1 with name 'b'",
        "
ALTER TABLE t
    DROP PARTITION ID '2026',
    MOVE PARTITION 2 TO TABLE u,
    FREEZE PARTITION 1 WITH NAME 'b'
",
    );
}

/// A comment between commands stays on its line; one on a line of its own
/// goes at the commands' indentation.
#[test]
fn comments_between_commands() {
    check(
        "alter table t\n  add column a Int8, -- new\n  -- old\n  drop column b",
        "
ALTER TABLE t
    ADD COLUMN a Int8,  -- new
    -- old
    DROP COLUMN b
",
    );
}

#[test]
fn delete_and_update_put_their_clauses_on_lines_of_their_own() {
    check(
        "delete from db.t on cluster c in partition 1 where a = 1 and b = 2 settings lightweight_deletes_sync = 0;\n\
         update t set a = 1, b = b + 1 where c = 3;",
        "
DELETE FROM db.t ON CLUSTER c
IN PARTITION 1
WHERE a = 1 AND b = 2
SETTINGS lightweight_deletes_sync = 0;

UPDATE t
SET a = 1, b = b + 1
WHERE c = 3;
",
    );
}
