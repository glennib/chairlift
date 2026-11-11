use crate::Options;

/// Formats `source` at `width`, checking that formatting again changes
/// nothing.
fn fmt_in(source: &str, jinja: bool, width: u16) -> String {
    let options = Options::default().with_jinja(jinja).with_line_width(width);
    let once = crate::format(source, &options).unwrap_or_else(|e| panic!("{source:?}: {e}"));
    let twice = crate::format(&once, &options).unwrap_or_else(|e| panic!("{once:?}: {e}"));
    assert_eq!(twice, once, "not idempotent on {source:?}");
    once
}

fn fmt(source: &str) -> String {
    fmt_in(source, false, 100)
}

/// Checks that `source` formats to `expected`, and `expected` to itself.
fn check(source: &str, expected: &str) {
    let expected = expected.strip_prefix('\n').unwrap_or(expected);
    assert_eq!(fmt(source), expected, "from {source:?}");
    assert_eq!(fmt(expected), expected);
}

fn check_jinja(source: &str, expected: &str) {
    let expected = expected.strip_prefix('\n').unwrap_or(expected);
    assert_eq!(fmt_in(source, true, 100), expected, "from {source:?}");
    assert_eq!(fmt_in(expected, true, 100), expected);
}

#[test]
fn the_column_list_has_its_bracket_alone_and_one_element_per_line() {
    check(
        "create or replace temporary table if not exists db.t uuid '1' on cluster c (a Int8) engine = Memory",
        "
CREATE OR REPLACE TEMPORARY TABLE IF NOT EXISTS db.t UUID '1' ON CLUSTER c
(
    a Int8
)
ENGINE = Memory
",
    );
    check(
        "create table t (a Int8, b Nullable(String) not null collate utf8, c String default 'x' codec(ZSTD(1)) statistics(tdigest) ttl d comment 'c' primary key,)",
        "
CREATE TABLE t
(
    a Int8,
    b Nullable(String) NOT NULL COLLATE utf8,
    c String DEFAULT 'x' CODEC(ZSTD(1)) STATISTICS(tdigest) TTL d COMMENT 'c' PRIMARY KEY,
)
",
    );
}

/// A column that does not fit breaks before each modifier but `NULL` and
/// `COLLATE`; a long comment is never split and may pass the width.
#[test]
fn a_long_column_breaks_before_its_modifiers() {
    check(
        "create table t (sero_time DateTime64(3, 'UTC') default inserted_at codec(Delta, ZSTD(1)) comment 'When the collector received the event, in UTC', \
         b String comment 'a comment long enough that the column does not fit on its line of one hundred', \
         c Nullable(String) null default 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx' codec(ZSTD(1)) ttl d + interval 1 day)",
        "
CREATE TABLE t
(
    sero_time DateTime64(3, 'UTC')
        DEFAULT inserted_at
        CODEC(Delta, ZSTD(1))
        COMMENT 'When the collector received the event, in UTC',
    b String
        COMMENT 'a comment long enough that the column does not fit on its line of one hundred',
    c Nullable(String) NULL
        DEFAULT 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'
        CODEC(ZSTD(1))
        TTL d + INTERVAL 1 DAY
)
",
    );
    // A type that does not fit breaks inside its brackets.
    assert_eq!(
        fmt_in(
            "create table t (t Tuple(a String, b UInt8) default (1, 2))",
            false,
            25
        ),
        "CREATE TABLE t\n(\n    t Tuple(\n        a String,\n        b UInt8\n    )\n        DEFAULT (1, 2)\n)\n"
    );
}

#[test]
fn indexes_projections_constraints_and_keys() {
    check(
        "create table t (a Int8, index i a type minmax granularity 1, index j (a, lower(s)) type bloom_filter(0.01), \
         projection p (select a, b order by a), projection q index a, b type basic with settings (index_granularity = 1), \
         constraint c check a > 0, constraint d assume a < 10, primary key (a), foreign key (a) references db.p (b) on delete cascade)",
        "
CREATE TABLE t
(
    a Int8,
    INDEX i a TYPE minmax GRANULARITY 1,
    INDEX j (a, lower(s)) TYPE bloom_filter(0.01),
    PROJECTION p (
        SELECT
            a,
            b
        ORDER BY a
    ),
    PROJECTION q INDEX a, b TYPE basic WITH SETTINGS (index_granularity = 1),
    CONSTRAINT c CHECK a > 0,
    CONSTRAINT d ASSUME a < 10,
    PRIMARY KEY (a),
    FOREIGN KEY (a) REFERENCES db.p(b) ON DELETE CASCADE
)
",
    );
    // An index or a projection index that does not fit breaks before `TYPE`,
    // `GRANULARITY` and `WITH SETTINGS`.
    check(
        "create table t (projection p_compact index region type basic with settings (min_bytes_for_wide_part = 1000000000))",
        "
CREATE TABLE t
(
    PROJECTION p_compact INDEX region
        TYPE basic
        WITH SETTINGS (min_bytes_for_wide_part = 1000000000)
)
",
    );
    check(
        "create table t (index idx_with_a_long_name lower(some_long_column_name) type ngrambf_v1(3, 256, 2, 0) granularity 4)",
        "
CREATE TABLE t
(
    INDEX idx_with_a_long_name lower(some_long_column_name)
        TYPE ngrambf_v1(3, 256, 2, 0)
        GRANULARITY 4
)
",
    );
}

#[test]
fn storage_clauses_go_one_per_line_in_the_order_written() {
    check(
        "create table t (a Int8) settings a = 1 engine = ReplicatedMergeTree('/p/{shard}', '{replica}') partition by toYYYYMM(d) \
         order by (a, b) primary key a sample by a unique key a comment 'c'",
        "
CREATE TABLE t
(
    a Int8
)
SETTINGS a = 1
ENGINE = ReplicatedMergeTree('/p/{shard}', '{replica}')
PARTITION BY toYYYYMM(d)
ORDER BY (a, b)
PRIMARY KEY a
SAMPLE BY a
UNIQUE KEY a
COMMENT 'c'
",
    );
    check(
        "create table t (a Int8) engine MergeTree order by tuple(a, b asc)",
        "
CREATE TABLE t
(
    a Int8
)
ENGINE MergeTree
ORDER BY tuple(a, b ASC)
",
    );
}

#[test]
fn keys_ttl_rules_and_settings_fit_or_go_one_per_line() {
    assert_eq!(
        fmt_in(
            "create table t (a Int8) engine = Memory order by (site desc, event_time) primary key (site, t) \
             settings a = 1, b = 2",
            false,
            30,
        ),
        "CREATE TABLE t\n(\n    a Int8\n)\nENGINE = Memory\nORDER BY (\n    site DESC,\n    event_time\n)\n\
         PRIMARY KEY (site, t)\nSETTINGS a = 1, b = 2\n"
    );
    assert_eq!(
        fmt_in(
            "create table t (d Date) engine = Memory order by d \
             ttl d + interval 1 month delete where x = 1, d + interval 2 month to disk 'a'",
            false,
            50,
        ),
        "CREATE TABLE t\n(\n    d Date\n)\nENGINE = Memory\nORDER BY d\nTTL\n    \
         d + INTERVAL 1 MONTH DELETE WHERE x = 1,\n    d + INTERVAL 2 MONTH TO DISK 'a'\n"
    );
    check(
        "create table t (d Date) engine = MergeTree order by d ttl d recompress codec(ZSTD(12)), d + 1 group by k set v = max(v)",
        "
CREATE TABLE t
(
    d Date
)
ENGINE = MergeTree
ORDER BY d
TTL d RECOMPRESS CODEC(ZSTD(12)), d + 1 GROUP BY k SET v = max(v)
",
    );
}

#[test]
fn a_ttl_rule_that_does_not_fit_breaks_before_where_group_by_and_set() {
    assert_eq!(
        fmt_in(
            "create table t (d Date) engine = MergeTree order by d \
             ttl d + interval 1 month delete where x = 1 and d > today() - 100, \
             d + interval 1 year to volume 'cold'",
            false,
            40,
        ),
        "CREATE TABLE t\n(\n    d Date\n)\nENGINE = MergeTree\nORDER BY d\nTTL\n    \
         d + INTERVAL 1 MONTH DELETE\n        WHERE x = 1\n            AND d > today() - 100,\n    \
         d + INTERVAL 1 YEAR TO VOLUME 'cold'\n"
    );
    assert_eq!(
        fmt_in(
            "create table t (d Date) engine = MergeTree order by d \
             ttl d + interval 1 month group by d set x = max(x), y = min(y)",
            false,
            40,
        ),
        "CREATE TABLE t\n(\n    d Date\n)\nENGINE = MergeTree\nORDER BY d\n\
         TTL d + INTERVAL 1 MONTH\n    GROUP BY d\n    SET x = max(x), y = min(y)\n"
    );
}

/// `AS` ends the line of the header, the element list or `EMPTY`, and
/// otherwise stands on a line of its own, where it cannot read as an alias;
/// the query starts the next line.
#[test]
fn as_a_query_starts_a_line_after_as() {
    check(
        "create table t engine = Memory as select a, b from u",
        "
CREATE TABLE t
ENGINE = Memory
AS
SELECT
    a,
    b
FROM u
",
    );
    check(
        "create table t (a Int8) engine = MergeTree order by tuple() settings a = 1 as select 1",
        "
CREATE TABLE t
(
    a Int8
)
ENGINE = MergeTree
ORDER BY tuple()
SETTINGS a = 1
AS
SELECT 1
",
    );
    check(
        "create table t empty as select 1",
        "CREATE TABLE t EMPTY AS\nSELECT 1\n",
    );
    check(
        "create table t engine = Memory empty as select 1",
        "CREATE TABLE t\nENGINE = Memory\nEMPTY AS\nSELECT 1\n",
    );
    check(
        "create table t as select 1",
        "CREATE TABLE t AS\nSELECT 1\n",
    );
    // A query alone in brackets hugs them; a union of them does not.
    check(
        "create table t engine = Memory as (select a, b from u)",
        "
CREATE TABLE t
ENGINE = Memory
AS (
    SELECT
        a,
        b
    FROM u
)
",
    );
    check(
        "create materialized view v to t as (select 1)",
        "CREATE MATERIALIZED VIEW v TO t AS (\n    SELECT 1\n)\n",
    );
    check(
        "create table t engine = Memory as (select 1) union all (select 2)",
        "
CREATE TABLE t
ENGINE = Memory
AS
(
    SELECT 1
)
UNION ALL
(
    SELECT 2
)
",
    );
}

#[test]
fn as_a_table_or_a_table_function_stays_on_the_header_line() {
    check(
        "create table t as db.t2 engine = Memory comment 'x'",
        "CREATE TABLE t AS db.t2\nENGINE = Memory\nCOMMENT 'x'\n",
    );
    check(
        "create table t as remote('host', db, t2, settings connect_timeout = 1)",
        "CREATE TABLE t AS remote('host', db, t2, SETTINGS connect_timeout = 1)\n",
    );
    check("create table t clone as t2", "CREATE TABLE t CLONE AS t2\n");
    // A header that does not fit breaks before them, one level in.
    assert_eq!(
        fmt_in("create table db.t as remote('host', db, t2)", false, 30),
        "CREATE TABLE db.t\n    AS remote('host', db, t2)\n"
    );
}

#[test]
fn a_materialized_view_breaks_its_header_before_to() {
    check(
        "create materialized view if not exists db.v on cluster c to db.t as select 1",
        "CREATE MATERIALIZED VIEW IF NOT EXISTS db.v ON CLUSTER c TO db.t AS\nSELECT 1\n",
    );
    check(
        "create materialized view analytics.page_views_per_hour_materialized_view_name to analytics.page_views_per_hour_target as select 1",
        "
CREATE MATERIALIZED VIEW analytics.page_views_per_hour_materialized_view_name
    TO analytics.page_views_per_hour_target AS
SELECT 1
",
    );
}

#[test]
fn a_materialized_view_with_columns_storage_and_other_clauses() {
    check(
        "create materialized view v to t (a Int8, b String) as select 1",
        "
CREATE MATERIALIZED VIEW v TO t
(
    a Int8,
    b String
) AS
SELECT 1
",
    );
    check(
        "create materialized view v (a Int8) engine = MergeTree order by a populate as select 1",
        "
CREATE MATERIALIZED VIEW v
(
    a Int8
)
ENGINE = MergeTree
ORDER BY a
POPULATE AS
SELECT 1
",
    );
    check(
        "create materialized view v to t definer = u@'%' sql security definer comment 'x' as select 1",
        "
CREATE MATERIALIZED VIEW v TO t
DEFINER = u@'%' SQL SECURITY DEFINER
COMMENT 'x'
AS
SELECT 1
",
    );
}

#[test]
fn a_refresh_clause_stays_on_the_header_line_if_it_fits() {
    check(
        "create materialized view v refresh every 1 hour append to t as select 1",
        "CREATE MATERIALIZED VIEW v REFRESH EVERY 1 HOUR APPEND TO t AS\nSELECT 1\n",
    );
    check(
        "create materialized view v refresh every 1 hour offset 5 minute randomize for 1 minute depends on a, db.b \
         settings refresh_retries = 1 append to t as select 1",
        "
CREATE MATERIALIZED VIEW v
    REFRESH EVERY 1 HOUR
        OFFSET 5 MINUTE
        RANDOMIZE FOR 1 MINUTE
        DEPENDS ON a, db.b
        SETTINGS refresh_retries = 1
        APPEND
    TO t AS
SELECT 1
",
    );
}

#[test]
fn time_series_targets_indent_their_storage_clauses() {
    check(
        "create table ts engine = TimeSeries data db.d tags inner engine = Memory recent samples inner columns (id UUID) \
         time ranges engine = MergeTree order by id metrics inner uuid '1'",
        "
CREATE TABLE ts
ENGINE = TimeSeries
DATA db.d
TAGS INNER
    ENGINE = Memory
RECENT SAMPLES INNER COLUMNS
(
    id UUID
)
TIME RANGES
    ENGINE = MergeTree
    ORDER BY id
METRICS INNER UUID '1'
",
    );
}

#[test]
fn comments_and_blank_lines_where_the_author_put_them() {
    check(
        "-- header\ncreate table t -- name\n( -- open\n  a Int8, -- a\n\n\n  -- b\n  b String /* inline */,\n  c String -- pinned\n  , d UInt8\n)\n\
         -- engine\nengine = MergeTree\n\n\n-- key\norder by a -- order\nsettings a = 1, -- one\n  b = 2",
        "
-- header
CREATE TABLE t  -- name
(  -- open
    a Int8,  -- a

    -- b
    b String /* inline */,
    c String  -- pinned
    , d UInt8
)
-- engine
ENGINE = MergeTree

-- key
ORDER BY a  -- order
SETTINGS
    a = 1,  -- one
    b = 2
",
    );
    // A comment before `AS` puts it on a line of its own and leaves the
    // header whole.
    check(
        "create table t empty\n-- the query\nas -- why\nselect 1",
        "CREATE TABLE t EMPTY\n-- the query\nAS  -- why\nSELECT 1\n",
    );
    check(
        "create materialized view v to t\n-- the query\nas -- why\nselect 1",
        "CREATE MATERIALIZED VIEW v TO t\n-- the query\nAS  -- why\nSELECT 1\n",
    );
}

#[test]
fn verbatim_items_and_clauses_keep_their_text() {
    check(
        "create table t (a Int8, b ?, c   Int8 ?? x  y,\n   d   Int8) engine = MergeTree order by a ?? b ttl d",
        "
CREATE TABLE t
(
    a Int8,
    b ?,
    c   Int8 ?? x  y,
    d Int8
)
ENGINE = MergeTree
order by a ?? b
TTL d
",
    );
}

#[test]
fn block_tags_between_elements_and_clauses() {
    check_jinja(
        "create table {{ db }}.events_{{ suffix }} on cluster {{ cluster }} (a Int8,\n{% if x %}\nb Int8,\n{% endif %}\nc Int8)\n\
         {% if prod %}\nengine = ReplicatedMergeTree('/clickhouse/{{ shard }}', '{replica}')\n{% else %}\nengine = MergeTree\n{% endif %}\n\
         order by a settings index_granularity = {{ granularity }}",
        "
CREATE TABLE {{ db }}.events_{{ suffix }} ON CLUSTER {{ cluster }}
(
    a Int8,
    {% if x %}
    b Int8,
    {% endif %}
    c Int8
)
{% if prod %}
ENGINE = ReplicatedMergeTree('/clickhouse/{{ shard }}', '{replica}')
{% else %}
ENGINE = MergeTree
{% endif %}
ORDER BY a
SETTINGS index_granularity = {{ granularity }}
",
    );
}
