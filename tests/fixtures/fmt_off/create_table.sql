-- A column list aligned by hand.
create table if not exists analytics.page_views (
-- fmt: off
    event_time   DateTime64(3, 'UTC')    CODEC(Delta, ZSTD),
    site         LowCardinality(String),
    visitor_id   UUID,
    url          String                  CODEC(ZSTD(3)),
    duration_ms  UInt32                  DEFAULT 0,
-- fmt: on
    index   url_idx url type bloom_filter granularity 4
) engine = MergeTree partition by toYYYYMM(event_time) order by (site, event_time) ttl toDateTime(event_time) + interval 1 year;
