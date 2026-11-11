-- Tear-down before a test run.
drop table if exists analytics.page_views on cluster '{cluster}' sync;
drop table if exists analytics.page_views_old, analytics.page_views_new, analytics.page_views_backfill_2025, analytics.page_views_backfill_2026;
DROP VIEW IF EXISTS analytics.active_sites;
drop dictionary if exists analytics.sites_dict;
drop database if exists scratch;
drop function if exists normalise_site;
drop temporary table if exists tmp_ids;

truncate table analytics.sessions;
truncate analytics.sessions_staging;
TRUNCATE ALL TABLES FROM IF EXISTS scratch LIKE 'tmp_%';

detach table analytics.sessions permanently;
attach table analytics.sessions;
detach dictionary analytics.sites_dict;
attach dictionary analytics.sites_dict;
undrop table analytics.sessions;

attach table analytics.sessions_restored from '/var/lib/clickhouse/restore/sessions' (
    session_id String,
    started_at DateTime
)
engine = MergeTree
order by session_id;
