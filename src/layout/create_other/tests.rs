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

#[test]
fn an_index_is_a_one_liner() {
    check(
        "create unique index if not exists idx on db.t on cluster c (a desc, b) type minmax granularity 1;\n\
         create index idx on t a + b;",
        "
CREATE UNIQUE INDEX IF NOT EXISTS idx ON db.t ON CLUSTER c (a DESC, b) TYPE minmax GRANULARITY 1;
CREATE INDEX idx ON t a + b;
",
    );
    check_at(
        "create index idx_events on analytics.events (event_date desc, user_id) type bloom_filter(0.01) granularity 4",
        "
CREATE INDEX idx_events ON analytics.events (event_date DESC, user_id)
    TYPE bloom_filter(0.01)
    GRANULARITY 4
",
        80,
    );
    check_at(
        "create index idx_events on analytics.events (event_date desc, user_id, session_id, site_id) type minmax",
        "
CREATE INDEX idx_events ON analytics.events (
    event_date DESC,
    user_id,
    session_id,
    site_id
)
    TYPE minmax
",
        60,
    );
}

#[test]
fn view_aliases_stay_on_the_header_line() {
    check(
        "create view v (a, b) as select 1, 2;\n\
         create view w (a Int64, b Int64) (x, y) as select 1, 2;\n\
         create definer = u sql security definer view d as select 1",
        "
CREATE VIEW v (a, b) AS
SELECT
    1,
    2;

CREATE VIEW w
(
    a Int64,
    b Int64
) (x, y) AS
SELECT
    1,
    2;

CREATE DEFINER = u SQL SECURITY DEFINER VIEW d AS
SELECT 1
",
    );
}

/// `ON CLUSTER` goes on a line of its own, one level in, when the header
/// does not fit.
#[test]
fn a_header_that_does_not_fit_breaks_before_on_cluster() {
    check_at(
        "create database if not exists analytics on cluster main engine = Atomic",
        "
CREATE DATABASE IF NOT EXISTS analytics
    ON CLUSTER main
ENGINE = Atomic
",
        50,
    );
    check_at(
        "create view analytics.recent on cluster main as select 1",
        "
CREATE VIEW analytics.recent
    ON CLUSTER main AS
SELECT 1
",
        40,
    );
}

/// Nested pairs glue their bracket to their key, as the source's name does.
#[test]
fn nested_pairs_glue_their_bracket() {
    check(
        "create dictionary d (a UInt64) primary key a \
         source(mysql(replica(host 'a' priority 1) db 'x')) layout(flat()) lifetime(0)",
        "
CREATE DICTIONARY d
(
    a UInt64
)
PRIMARY KEY a
SOURCE(mysql(replica(host 'a' priority 1) db 'x'))
LAYOUT(flat())
LIFETIME(0)
",
    );
}
