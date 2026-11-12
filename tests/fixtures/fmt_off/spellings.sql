-- A `#` comment is a marker too, and case does not matter.
select site,
# fmt: off
    views   +   clicks   AS actions,
# FMT: ON
    visitors from analytics.daily_views;

select site,
--FMT:OFF
    views   *   2   AS doubled,
-- Fmt: On
    visitors from analytics.daily_views;

-- Comments that only look like markers.
select site,
-- fmt: off for now
    views   +   clicks   as actions,
--  fmt: off
    visitors from analytics.daily_views;

-- Marker text in strings and in block comments.
select '-- fmt: off' as s, 'a
-- fmt: off
b' as multiline, /* -- fmt: off */ views   +   clicks  as actions
from analytics.daily_views;

/*
-- fmt: off
*/
select   site  from analytics.sites;

-- A lone fmt: on is a comment.
select   site,
-- fmt: on
    views from analytics.daily_views;

-- A fmt: off inside a region needs a fmt: on of its own.
select site,
-- fmt: off
    views     AS v,
-- fmt: off
    clicks    AS c,
-- fmt: on
    visitors  AS u,
-- fmt: on
    sessions as s from analytics.daily_views;
