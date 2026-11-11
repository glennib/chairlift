//! The examples of the layout proposal
//! (`docs/spikes/2026-11-10-layout-proposal.md`) as the proposal prints
//! them, in its recommended alternative, for the tests that check a layout
//! reproduces them byte for byte. Each starts with a line break, which the
//! tests strip.
//!
//! The file is included with `#[path]` by the tests that use it.

#![allow(dead_code, reason = "each includer uses part of it")]

// Section 2a: line width. The same query at 120, 100 and 80 columns.

pub const WIDTH_120: &str = "
SELECT
    toStartOfHour(event_time) AS hour,
    round(100 * countIf(event_type = 'subscribe') / nullIf(countIf(event_type = 'page_view'), 0), 2) AS conversion_pct,
    arrayStringConcat(arraySlice(groupUniqArray(referrer_domain), 1, 5), ', ') AS top_referrer_domains
FROM raw.events
GROUP BY hour
";

pub const WIDTH_100: &str = "
SELECT
    toStartOfHour(event_time) AS hour,
    round(
        100 * countIf(event_type = 'subscribe') / nullIf(countIf(event_type = 'page_view'), 0),
        2
    ) AS conversion_pct,
    arrayStringConcat(
        arraySlice(groupUniqArray(referrer_domain), 1, 5),
        ', '
    ) AS top_referrer_domains
FROM raw.events
GROUP BY hour
";

pub const WIDTH_80: &str = "
SELECT
    toStartOfHour(event_time) AS hour,
    round(
        100 * countIf(event_type = 'subscribe')
            / nullIf(countIf(event_type = 'page_view'), 0),
        2
    ) AS conversion_pct,
    arrayStringConcat(
        arraySlice(groupUniqArray(referrer_domain), 1, 5),
        ', '
    ) AS top_referrer_domains
FROM raw.events
GROUP BY hour
";

// Section 2b: lists.

pub const IN_LIST: &str = "
DELETE FROM raw.events
WHERE event_type IN (
    'page_view',
    'article_view',
    'video_start',
    'video_complete',
    'scroll_25',
    'scroll_50',
    'scroll_75',
    'scroll_100',
    'share',
    'comment',
    'subscribe',
    'login'
)
";

pub const ARRAY_LITERAL: &str = "
WITH
    [
        1024,
        4096,
        16384,
        65536,
        262144,
        1048576,
        4194304,
        16777216,
        67108864,
        268435456,
        1073741824
    ] AS size_buckets_bytes
SELECT arrayMap(b -> formatReadableSize(b), size_buckets_bytes) AS labels
";

pub const SETTINGS_LIST: &str = "
CREATE TABLE landing.events
(
    event_time DateTime64(3, 'UTC')
)
ENGINE = MergeTree
ORDER BY event_time
SETTINGS
    ttl_only_drop_parts = 1,
    merge_with_ttl_timeout = 900,
    min_bytes_for_wide_part = 33554432,
    index_granularity = 8192,
    storage_policy = 'hot_cold',
    allow_nullable_key = 1
";

pub const ORDER_BY_FITS: &str = "
CREATE TABLE analytics.teaser_clicks
(
    site LowCardinality(String),
    sero_time DateTime('UTC'),
    product_feature LowCardinality(String),
    content_id String
)
ENGINE = SummingMergeTree
ORDER BY (site, sero_time, product_feature, content_id)
";

pub const ORDER_BY_BREAKS: &str = "
CREATE TABLE analytics.teaser_clicks_by_variant
(
    site LowCardinality(String),
    sero_time DateTime('UTC'),
    content_id String
)
ENGINE = SummingMergeTree
ORDER BY (
    site,
    toStartOfHour(sero_time),
    payload_product_feature,
    payload_data_data_teaser_id,
    payload_data_data_variant_id
)
";

pub const SELECT_LIST_SPLITS: &str = "
SELECT
    database,
    table,
    name,
    reason
FROM system.detached_parts
";

// Section 2c: conditions and commas.

pub const CONDITIONS_HANG: &str = "
SELECT count()
FROM raw.events
WHERE event_date >= today() - 7
    AND event_type IN ('page_view', 'article_view')
    AND site != ''
    AND NOT is_bot
";

pub const TRAILING_COMMAS: &str = "
SELECT
    toStartOfHour(event_time) AS hour,
    site,
    count() AS views
FROM raw.events
GROUP BY hour, site
";

// Section 2d: column definitions, not aligned.

pub const COLUMNS_NOT_ALIGNED: &str = "
CREATE TABLE analytics.sessions
(
    session_id UUID,
    site LowCardinality(String) COMMENT 'Publication slug',
    started_at DateTime64(3, 'UTC') CODEC(Delta, ZSTD(1)) COMMENT 'First event of the session',
    duration_ms UInt32 CODEC(T64, ZSTD(1)),
    is_logged_in Bool DEFAULT false
)
ENGINE = MergeTree
ORDER BY (site, started_at)
";

// Section 2e: blank lines.

pub const BLANK_LINES_KEPT: &str = "
-- The writer role: inserts into landing.events and nothing else.

CREATE ROLE events_writer;
GRANT INSERT ON landing.events TO events_writer;
GRANT events_writer TO events_consumer;


ALTER USER events_consumer DEFAULT ROLE ALL;
";

/// Not an example of the proposal: the rule that a statement on several
/// lines gets a blank line on each side.
pub const BLANK_AROUND_MULTILINE: &str = "
SELECT 1;

SELECT
    a,
    b
FROM t;

SELECT 2;
";

// Section 2f: comments.

pub const COLUMN_COMMENTS: &str = "
CREATE TABLE analytics.page_views_1h
(
    -- Start of the hour, in UTC.
    hour DateTime('UTC'),
    site LowCardinality(String),  -- the publication's slug
    views UInt64
)
-- Partial aggregates merge on read; query with sum and uniqMerge.
ENGINE = AggregatingMergeTree
ORDER BY (site, hour)
";

pub const LINE_COMMENT_BREAKS: &str = "
SELECT count()
FROM raw.events
ORDER BY (
    site,  -- most queries filter on one site
    event_time
)
";

pub const PINNED_COMMA: &str = "
SELECT
    toStartOfHour(event_time) AS hour,
    site  -- the publication's slug
    , count() AS views
FROM raw.events
GROUP BY hour, site
";

pub const CONDITION_COMMENTS: &str = "
SELECT count()
FROM raw.events
WHERE event_date >= today() - 7
    -- Bots inflate page views by about a third.
    AND NOT is_bot
    AND site != ''  -- empty for events from the test harness
";

/// Not an example of the proposal: `;` after a trailing line comment.
pub const SEMICOLON_ALONE: &str = "
SELECT 1  -- one
;
";

// Section 2g: Jinja.

pub const BLOCK_TAGS: &str = "
SELECT count()
FROM raw.events
WHERE true
    AND event_type = 'page_view'
    {% if this.cutoff is defined -%}
    AND event_time >= {{ this.cutoff | sql_datetime }}
    {% endif -%}
";

pub const FOR_LOOP: &str = "
SELECT
    site,
    {% for e in this.event_types %}
    countIf(event_type = '{{ e }}') AS {{ e }}_count,
    {% endfor %}
    count() AS total
FROM raw.events
GROUP BY site
";

// Section 2h: nesting.

pub const SUBQUERY_HUGS: &str = "
SELECT
    site,
    avg(views) AS avg_daily_views
FROM (
    SELECT
        site,
        toDate(event_time) AS day,
        count() AS views
    FROM raw.events
    GROUP BY site, day
) AS daily
GROUP BY site
";

pub const IN_SUBQUERY_HUGS: &str = "
SELECT count()
FROM raw.events
WHERE visitor_id IN (
    SELECT visitor_id
    FROM raw.subscriptions
    WHERE status = 'active'
)
";

pub const SINGLE_CTE_HUGS: &str = "
WITH daily AS (
    SELECT
        site,
        toDate(event_time) AS day
    FROM raw.events
)
SELECT
    site,
    count() AS days
FROM daily
GROUP BY site
";

pub const MATERIALIZED_VIEW: &str = "
CREATE MATERIALIZED VIEW analytics.events_daily_mv TO analytics.events_daily AS
SELECT
    toDate(event_time) AS day,
    event_name,
    count() AS cnt
FROM raw.events
GROUP BY day, event_name
";

pub const CASE_EXPANDS: &str = "
SELECT
    CASE
        WHEN duration_ms < 10000 THEN 'bounce'
        WHEN duration_ms < 60000 THEN 'skim'
        WHEN duration_ms < 300000 THEN 'read'
        ELSE 'deep read'
    END AS engagement,
    count() AS sessions
FROM analytics.sessions
GROUP BY engagement
";

pub const LAMBDA_WHOLE: &str = "
SELECT
    arrayFilter(
        (url, ts) -> ts >= session_start AND NOT startsWith(url, 'https://internal.example.com/'),
        urls,
        timestamps
    ) AS external_urls
FROM analytics.sessions
";

/// Not an example of the proposal: the same lambda at 90 columns, where its
/// body breaks after `->`.
pub const LAMBDA_BREAKS_90: &str = "
SELECT
    arrayFilter(
        (url, ts) ->
            ts >= session_start AND NOT startsWith(url, 'https://internal.example.com/'),
        urls,
        timestamps
    ) AS external_urls
FROM analytics.sessions
";

pub const JOIN_ON: &str = "
SELECT
    o.order_id,
    p.price
FROM raw.orders AS o
ASOF LEFT JOIN dim.prices AS p
    ON p.sku = o.sku
    AND p.currency = o.currency
    AND o.ordered_at >= p.valid_from
";

pub const INSERT_SELECT: &str = "
INSERT INTO analytics.events_daily (day, event_name, cnt)
SELECT
    toDate(event_time) AS day,
    event_name,
    count() AS cnt
FROM raw.events
WHERE event_time < '2026-01-01 00:00:00'
GROUP BY day, event_name
";

pub const ALTER_TABLE: &str = "
ALTER TABLE raw.events ADD COLUMN IF NOT EXISTS element String AFTER duration_ms;

ALTER TABLE analytics.page_views_1h
    ADD COLUMN referrer LowCardinality(String) DEFAULT '' AFTER site,
    MODIFY TTL hour + INTERVAL 180 DAY;
";

pub const GRANT: &str = "
GRANT SELECT ON analytics.page_views_1h TO reader;

GRANT SELECT, INSERT, ALTER UPDATE, ALTER DELETE
    ON analytics.page_views_1h_with_a_long_name
    TO analytics_writer, backfill_job, maintenance
    WITH GRANT OPTION;
";

// Section 3: ClickHouse-specific constructs.

pub const PROJECTION_AND_INDEX: &str = "
CREATE TABLE analytics.page_views ON CLUSTER '{cluster}'
(
    event_time DateTime64(3, 'UTC') CODEC(Delta, ZSTD(1)),
    visitor_id String COMMENT 'Anonymous, 16 hex digits',
    INDEX visitor_bf visitor_id TYPE bloom_filter(0.01) GRANULARITY 4,
    PROJECTION by_visitor (
        SELECT *
        ORDER BY visitor_id
    )
)
ENGINE = ReplicatedMergeTree
ORDER BY tuple()
SETTINGS index_granularity = 8192
COMMENT 'One row per page view'
";

pub const STORAGE_CLAUSES: &str = "
CREATE TABLE analytics.visits
(
    site LowCardinality(String),
    event_time DateTime,
    visitor_id String
)
ENGINE = MergeTree
PRIMARY KEY (site, toStartOfHour(event_time))
ORDER BY (site, toStartOfHour(event_time), cityHash64(visitor_id))
SAMPLE BY cityHash64(visitor_id)
TTL event_time + INTERVAL 13 MONTH DELETE
SETTINGS ttl_only_drop_parts = 1
";

pub const QUERY_CLAUSES: &str = "
SELECT
    site,
    tag,
    count() AS views
FROM raw.articles
ARRAY JOIN tags AS tag
PREWHERE event_date = today()
WHERE event_type = 'page_view'
GROUP BY site, tag WITH TOTALS
ORDER BY views DESC
LIMIT 3 BY site
LIMIT 100
SETTINGS max_threads = 8
FORMAT JSONEachRow
";

pub const ATOMS: &str = "
SELECT
    * EXCEPT (raw_payload),
    status::UInt16 AS status_code,
    {limit:UInt32} AS row_limit
FROM raw.requests
WHERE event_time >= {since:DateTime} - INTERVAL 1 DAY
";
