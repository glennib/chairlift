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
fn a_dictionary_is_laid_out_as_a_table() {
    check(
        "create dictionary analytics.sites_dict (site_id UInt64, name String default '', parent UInt64 hierarchical) \
         primary key site_id source(clickhouse(table 'sites' db 'analytics')) layout(hashed()) lifetime(min 0 max 3600)",
        "
CREATE DICTIONARY analytics.sites_dict
(
    site_id UInt64,
    name String DEFAULT '',
    parent UInt64 HIERARCHICAL
)
PRIMARY KEY site_id
SOURCE(clickhouse(table 'sites' db 'analytics'))
LAYOUT(hashed())
LIFETIME(min 0 max 3600)
",
    );
}

/// A source that does not fit puts its pairs one per line, and the
/// password stays as written.
#[test]
fn a_long_source_breaks_its_pairs() {
    check(
        "CREATE DICTIONARY d (id UInt64) PRIMARY KEY id SOURCE(CLICKHOUSE(HOST 'clickhouse.internal' PORT 9000 \
         USER 'reader' PASSWORD 'p@ss;word' DB 'analytics' TABLE 'sites' INVALIDATE_QUERY 'SELECT max(updated_at) FROM sites')) \
         LAYOUT(COMPLEX_KEY_HASHED(SHARDS 4)) RANGE(MIN starts MAX ends) LIFETIME(300) SETTINGS(format_csv_allow_single_quotes = 0) COMMENT 'x'",
        "
CREATE DICTIONARY d
(
    id UInt64
)
PRIMARY KEY id
SOURCE(CLICKHOUSE(
    HOST 'clickhouse.internal'
    PORT 9000
    USER 'reader'
    PASSWORD 'p@ss;word'
    DB 'analytics'
    TABLE 'sites'
    INVALIDATE_QUERY 'SELECT max(updated_at) FROM sites'
))
LAYOUT(COMPLEX_KEY_HASHED(SHARDS 4))
RANGE(MIN starts MAX ends)
LIFETIME(300)
SETTINGS(format_csv_allow_single_quotes = 0)
COMMENT 'x'
",
    );
}

#[test]
fn a_view_is_laid_out_as_a_materialized_view() {
    check(
        "create or replace view analytics.recent as select site, count() as views from raw.events group by site",
        "
CREATE OR REPLACE VIEW analytics.recent AS
SELECT
    site,
    count() AS views
FROM raw.events
GROUP BY site
",
    );
    check(
        "create view v (a UInt8) sql security invoker as select 1 as a comment 'x'",
        "
CREATE VIEW v
(
    a UInt8
)
SQL SECURITY INVOKER
AS
SELECT 1 AS a
COMMENT 'x'
",
    );
}

#[test]
fn databases_and_functions_are_one_liners() {
    check(
        "create database if not exists analytics engine = Atomic comment 'x';\n\
         create function plus_one as x -> x + 1;",
        "
CREATE DATABASE IF NOT EXISTS analytics ENGINE = Atomic COMMENT 'x';
CREATE FUNCTION plus_one AS x -> x + 1;
",
    );
    check_at(
        "create database analytics on cluster '{cluster}' engine = Replicated('/clickhouse/databases/analytics', '{shard}', '{replica}') comment 'Analytics'",
        "
CREATE DATABASE analytics ON CLUSTER '{cluster}'
ENGINE = Replicated('/clickhouse/databases/analytics', '{shard}', '{replica}')
COMMENT 'Analytics'
",
        80,
    );
}
