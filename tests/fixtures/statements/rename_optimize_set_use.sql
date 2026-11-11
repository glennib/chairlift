use analytics;

set max_threads = 8, max_memory_usage = 20000000000;
SET send_logs_level = 'fatal';
set allow_experimental_analyzer;
set time zone 'Europe/Oslo';

-- Swap the backfilled table in.
rename table analytics.page_views to analytics.page_views_old, analytics.page_views_backfill to analytics.page_views on cluster '{cluster}';
exchange tables analytics.sessions and analytics.sessions_staging;
rename database scratch to scratch_old;

optimize table analytics.page_views final;
optimize table analytics.page_views partition '2026-01' final deduplicate by site, event_time, visitor_id;
OPTIMIZE TABLE analytics.sessions PARTITION ID '202601' FINAL SETTINGS optimize_throw_if_noop = 1;
