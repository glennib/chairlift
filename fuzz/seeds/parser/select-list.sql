2SELECT a + 1 AS b, [1, 2], (x, y), CASE WHEN a THEN b ELSE c END, INTERVAL 1 DAY, cast(x AS String), x::Int8, * EXCEPT (d) APPLY sum, COLUMNS('^a') REPLACE (a + 1 AS a)
