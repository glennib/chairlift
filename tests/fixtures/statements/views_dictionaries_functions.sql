create database if not exists analytics on cluster '{cluster}' engine = Atomic comment 'Aggregates for the dashboards';

create or replace view analytics.recent_views as select site, count() as views from raw.events where event_time > now() - interval 1 hour group by site;

CREATE VIEW analytics.views_since (site String, views UInt64)
AS SELECT site, count() AS views FROM raw.events WHERE event_time > {since:DateTime} GROUP BY site
COMMENT 'Parameterized: SELECT * FROM analytics.views_since(since = now() - 3600)';

create dictionary analytics.sites_dict (
  site_id UInt64,
  name String default '',
  parent_id UInt64 default 0 hierarchical,
  timezone String expression lower(tz)
)
primary key site_id
source(clickhouse(host 'localhost' port 9000 user 'reader' password 'kept;as-written' db 'analytics' table 'sites'))
layout(hashed())
lifetime(min 300 max 3600)
comment 'Sites by id';

CREATE DICTIONARY analytics.ip_ranges (prefix String, site_id UInt64) PRIMARY KEY prefix SOURCE(FILE(path 'ranges.tsv' format 'TabSeparated')) LAYOUT(IP_TRIE) LIFETIME(0);

create function clamp_views as (views, cap) -> if(views > cap, cap, views);
CREATE OR REPLACE FUNCTION site_key ON CLUSTER '{cluster}' AS site -> lower(replaceAll(site, ' ', '_'));

create view analytics.top_sites (site, views) as select site, count() from raw.events group by site;
CREATE OR REPLACE DEFINER = reporter SQL SECURITY DEFINER VIEW analytics.site_totals (site String, views UInt64) (s, v) AS SELECT site, count() FROM raw.events GROUP BY site;

create index idx_site on raw.events (site desc, event_time);
CREATE UNIQUE INDEX IF NOT EXISTS idx_user ON raw.events ON CLUSTER '{cluster}' user_id TYPE bloom_filter(0.01) GRANULARITY 4;
drop index if exists idx_site on raw.events;
