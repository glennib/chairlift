-- One command, as a migration adds a column.
alter table raw.events add column if not exists element String after duration_ms;

ALTER TABLE analytics.page_views_1h ON CLUSTER '{cluster}'
  ADD COLUMN referrer LowCardinality(String) DEFAULT '' AFTER site,
  modify ttl hour + interval 180 day;

-- A list of settings or TTL rules reads on after a comma, so it comes last.
alter table analytics.page_views_1h modify setting ttl_only_drop_parts = 1, merge_with_ttl_timeout = 3600;

alter table raw.events
    modify column visitor_id String default '' codec(ZSTD(1)) comment 'Anonymous, 16 hex digits',
    -- the index the dashboards filter on
    add index visitor_bf visitor_id type bloom_filter(0.01) granularity 4,
    add projection by_visitor (select * order by visitor_id),
    drop column if exists legacy_id,
    rename column ts to event_time;

alter table raw.events materialize index visitor_bf in partition 202601, materialize projection by_visitor;

ALTER TABLE raw.events DELETE WHERE event_type = 'test' SETTINGS mutations_sync = 2;
alter table raw.events update site = lower(site), event_type = replaceAll(event_type, '-', '_') where site != lower(site);

alter table raw.events drop partition 202512, detach partition id '202511', attach partition 202511 from raw.events_backfill;
alter table raw.events move partition 202510 to volume 'cold', freeze partition 202601 with name 'before-backfill';

alter table analytics.page_views_mv modify query select toStartOfHour(event_time) as hour, site, count() as views from raw.events group by hour, site;

ALTER DATABASE analytics MODIFY COMMENT 'Aggregates for the dashboards';

delete from raw.events where event_type in ('page_view', 'article_view', 'video_start', 'video_complete', 'scroll_25', 'scroll_50', 'scroll_75', 'scroll_100');
DELETE FROM raw.events ON CLUSTER '{cluster}' IN PARTITION 202601 WHERE site = 'test' SETTINGS lightweight_deletes_sync = 0;
update raw.events set site = 'nordlys', event_type = 'page_view' where site = 'Nordlys' ;
