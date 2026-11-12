select  site, count() as views from raw.page_views group by site;

-- Without a fmt: on, the rest of the file is kept.
-- fmt: off
SELECT   site,
         uniq(visitor_id)   AS visitors
FROM     raw.page_views
GROUP BY site;

select   1  as one;
