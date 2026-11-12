-- A region that starts in one statement and ends in the next.
select  site, count() as views from raw.page_views
-- fmt: off
GROUP BY   site
ORDER BY   views DESC;

SELECT   site,   uniq(visitor_id)   AS visitors
-- fmt: on
from raw.page_views group by site;

-- Markers at the end of lines of code keep those lines.
select event_date,  site, -- fmt: off
    sumIf(views, device = 'mobile')   AS mobile,
    sumIf(views, device = 'desktop')  AS desktop, -- fmt: on
    sum(views) as total from analytics.daily_views group by event_date, site;

select 1 as one, 2 as two ; -- fmt: off
SELECT    3 AS three; -- fmt: on
select  4 as four;
