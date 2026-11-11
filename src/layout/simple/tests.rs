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
fn statements_that_fit_stay_on_one_line() {
    check(
        "drop  table if exists\n  db.events on cluster '{cluster}' sync;\ntruncate table db.t;\n\
         detach table t permanently;\nattach table t;\nundrop table t;\n\
         rename table a to b, c to d;\nexchange tables a and b;\n\
         optimize table t partition tuple() final deduplicate by a, b;\n\
         set max_threads = 1, send_logs_level = 'fatal';\nuse system;",
        "
DROP TABLE IF EXISTS db.events ON CLUSTER '{cluster}' SYNC;
TRUNCATE TABLE db.t;
DETACH TABLE t PERMANENTLY;
ATTACH TABLE t;
UNDROP TABLE t;
RENAME TABLE a TO b, c TO d;
EXCHANGE TABLES a AND b;
OPTIMIZE TABLE t PARTITION tuple() FINAL DEDUPLICATE BY a, b;
SET max_threads = 1, send_logs_level = 'fatal';
USE system;
",
    );
}

/// A list that does not fit goes one item per line, and what follows it
/// starts a line of its own.
#[test]
fn a_list_that_does_not_fit_goes_one_item_per_line() {
    check_at(
        "set max_threads = 8, max_memory_usage = 10000000000, send_logs_level = 'fatal'",
        "
SET
    max_threads = 8,
    max_memory_usage = 10000000000,
    send_logs_level = 'fatal'
",
        60,
    );
    check_at(
        "drop table if exists analytics.page_views, analytics.sessions on cluster '{cluster}' sync",
        "
DROP TABLE IF EXISTS
    analytics.page_views,
    analytics.sessions
ON CLUSTER '{cluster}' SYNC
",
        60,
    );
    check_at(
        "rename table analytics.page_views to analytics.page_views_old, analytics.page_views_new to analytics.page_views",
        "
RENAME TABLE
    analytics.page_views TO analytics.page_views_old,
    analytics.page_views_new TO analytics.page_views
",
        60,
    );
}

/// Output clauses start lines of their own when the statement does not
/// fit.
#[test]
fn output_clauses_break_as_a_query_s_do() {
    check(
        "drop table t format Null settings max_execution_time = 1",
        "DROP TABLE t FORMAT Null SETTINGS max_execution_time = 1\n",
    );
    check_at(
        "optimize table analytics.page_views final settings optimize_throw_if_noop = 1",
        "
OPTIMIZE TABLE analytics.page_views FINAL
SETTINGS optimize_throw_if_noop = 1
",
        60,
    );
}

/// The definition after `ATTACH`'s name is kept as written.
#[test]
fn an_attach_definition_is_kept_as_written() {
    check(
        "attach table t\n(\n  a UInt8\n)\nengine = Memory",
        "
ATTACH TABLE t (
  a UInt8
)
engine = Memory
",
    );
}

/// Comments between the parts stay where they are; a statement they break
/// is set apart by blank lines.
#[test]
fn comments_between_the_parts() {
    check(
        "drop table t -- gone\n;\nset a = 1, -- first\nb = 2;",
        "
DROP TABLE t  -- gone
;

SET
    a = 1,  -- first
    b = 2;
",
    );
}
