-- A hand-written query kept between two statements that are formatted.
select site_id,name from analytics.sites where active=1 ;

-- fmt: off
SELECT   'nordlys'     AS site,   1 AS id
UNION ALL
SELECT   'fjordposten' AS site,  22 AS id;
-- fmt: on

select  site , count()  as views from raw.page_views group by site order by views desc limit 10;

-- Part of a SELECT list.
select event_date, site,
-- fmt: off
    sumIf(views,   device = 'mobile')  AS mobile_views,
    sumIf(views,   device = 'desktop') AS desktop_views,
    sumIf(views,   device = 'tablet')  AS tablet_views,
-- fmt: on
    sum(views) as total_views
from analytics.daily_views where event_date >= today() - 7 group by event_date, site;

-- The branches of a CASE.
select visitor_id, case
-- fmt: off
        when sessions >= 100 then 'loyal'
        when sessions >=  10 then 'regular'
        when sessions >=   1 then 'casual'
-- fmt: on
        else 'unknown' end as segment
from analytics.visitors;

-- An IN list laid out as a grid.
select * from raw.events where event_type in (
-- fmt: off
    'page_view',  'scroll_25',  'scroll_50',
    'scroll_75',  'scroll_100', 'click'
-- fmt: on
) and site = 'nordlys';
