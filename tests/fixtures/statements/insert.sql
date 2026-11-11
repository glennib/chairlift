-- Rows inline, as a test or a seed script writes them.
insert into analytics.sites (site_id, name, timezone) values
    (1, 'Nordlys', 'Europe/Oslo'),
    (2, 'Fjordposten', 'Europe/Oslo');

insert into analytics.sites values (3,'Kystavisa','Europe/Oslo') ;

INSERT INTO analytics.sites SETTINGS async_insert = 1, wait_for_async_insert = 0 VALUES (4, 'Vestlandet', 'Europe/Oslo');

-- Data in another format runs to the first blank line.
insert into raw.events format JSONEachRow
{"event_time": "2026-01-01 00:00:00", "event_type": "page_view", "site": "nordlys"}
{"event_time": "2026-01-01 00:00:01", "event_type":   "scroll_25", "site": "nordlys"}

insert into raw.events (event_time, event_type, site) format CSV
2026-01-01 00:00:02,page_view,  fjordposten

INSERT INTO analytics.events_daily (day, event_name, cnt) SELECT toDate(event_time) AS day, event_name, count() AS cnt FROM raw.events WHERE event_time < '2026-01-01 00:00:00' GROUP BY day, event_name;

with recent as (select * from raw.events where event_time > now() - interval 1 day)
insert into analytics.recent_sites (site, views)
select site, count() from recent group by site settings max_threads = 4;

insert into table function file('sites.csv', 'CSVWithNames') select * from analytics.sites;

insert into raw.events from infile 'events.csv.gz' compression 'gzip' format CSV;

insert into analytics.page_views_by_site_and_hour (site, hour, page_views, unique_visitors, sessions, bounce_rate)
select site, toStartOfHour(event_time), count(), uniq(visitor_id), uniq(session_id), avg(bounced) from raw.page_views group by all;
