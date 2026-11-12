-- The rows of an INSERT, the region starting at VALUES.
insert into analytics.sites (site_id, name, timezone)
-- fmt: off
VALUES (  1, 'Nordlys',     'Europe/Oslo'),
       ( 22, 'Fjordposten', 'Europe/Oslo'),
       (333, 'Kystavisa',   'Europe/Oslo');
-- fmt: on

-- A whole INSERT in a region.
-- fmt: off
INSERT INTO analytics.sites VALUES
    (4,  'Vestlandet', 'Europe/Oslo'),
    (55, 'Nordkappen', 'Europe/Oslo');
-- fmt: on

-- The rows of the values() table function.
select   s.name, l.label from analytics.sites as s inner join values('id UInt32, label String',
-- fmt: off
    ( 1, 'one'        ),
    (22, 'twenty-two' )
-- fmt: on
) as l on s.site_id = l.id;
