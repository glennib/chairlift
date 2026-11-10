# chairlift layout proposal

All SQL below parses with `clickhouse-format -n` 26.10.
For blocks with Jinja, that means after stripping `{% %}` lines and replacing `{{ }}` with an identifier.
`validate.sh` in this directory re-checks every block.
Within each question, the alternatives produce the same `--oneline` canonical form, so they differ only in whitespace.
Default width in the examples is 100.

## 1. Baseline: clickhouse-format on the DESIGN.md example

I stripped the Jinja (the `{% if %}` block became a plain `and event_time >= '…'`, the role became `reader`)
and ran `clickhouse-format -n`:

```sql
CREATE TABLE analytics.page_views_1h
(
    `hour` DateTime('UTC') CODEC(Delta, ZSTD(1)),
    `site` LowCardinality(String),
    `views` UInt64,
    `visitors` AggregateFunction(uniq, String)
)
ENGINE = AggregatingMergeTree
PARTITION BY toStartOfDay(hour)
ORDER BY (site, hour)
TTL hour + toIntervalDay(90)
SETTINGS ttl_only_drop_parts = 1
;

CREATE MATERIALIZED VIEW analytics.page_views_1h_mv TO analytics.page_views_1h
AS SELECT
    toStartOfHour(event_time) AS hour,
    site,
    count() AS views,
    uniqState(visitor_id) AS visitors
FROM raw.events
WHERE true AND (event_type = 'page_view') AND (event_time >= '2026-01-01 00:00:00')
GROUP BY
    hour,
    site
;

GRANT SELECT ON analytics.page_views_1h TO reader;
```

`--max_line_length N` exists but is not a wrap width.
It prints a whole statement on one line if the one-line form is shorter than N, and otherwise changes nothing.
At 80, 100 and 120 the output above is byte-identical;
`select a, b from t where x = 1 group by a, b` collapses to one line at 80
and stays expanded at 40. clickhouse-format has no width-aware wrapping at all.

**Worth keeping:** one clause per line; `(` alone before the column list; four-space indent; one SELECT item per line;
`ENGINE = X`; table `COMMENT` as its own last clause; `ARRAY JOIN`, `PREWHERE`,
`LIMIT n BY` and `FORMAT` each on their own line.

**Reads badly, but is whitespace and so is chairlift's to fix:**

- No width awareness.
  The WHERE line above is 83 columns.
  A six-entry query `SETTINGS` came out as one 154-column line,
  and a WHERE with a 12-literal `IN` list and a 27-element array as one 207-column line.
- `GROUP BY` puts two short keys on two lines.
- `;` sits alone on a line (`--semicolons_inline` turns that off).
- `AS SELECT` starts a line, and `INSERT INTO t (a, b) SELECT` keeps the first select item's line with the INSERT.
- A subquery puts `FROM` and `(` on separate lines; a CTE puts `name AS` and `(` on separate lines.
- `WITH TOTALS` is indented under the GROUP BY keys.
- Comments inside statements and all blank lines are dropped, even with `--comments`.

**Token rewrites chairlift can't make:** backquoted column names
(column list only),
`INTERVAL 90 DAY` → `toIntervalDay(90)`, `x ? 1 : 2` → `if(x, 1, 2)`, `CASE` → `multiIf`,
`x::UInt8` → `CAST(x, 'UInt8')`, `* EXCEPT (a)` → `* EXCEPT a`, `ORDER BY site` → `ORDER BY site ASC`,
dropped `TTL … DELETE`, and added parentheses around WHERE/ON conjuncts and around each `ALTER` command.
So chairlift can match clickhouse-format's shape, which people know from `SHOW CREATE TABLE`, but never its text.

## 2. Open style questions

### a. Line width

The same SELECT at 120, 100 and 80.
The rule in all three: a call's arguments stay on one line if they fit, otherwise one per line;
a binary expression that still doesn't fit breaks before the operator.

```sql
SELECT
    toStartOfHour(event_time) AS hour,
    round(100 * countIf(event_type = 'subscribe') / nullIf(countIf(event_type = 'page_view'), 0), 2) AS conversion_pct,
    arrayStringConcat(arraySlice(groupUniqArray(referrer_domain), 1, 5), ', ') AS top_referrer_domains
FROM raw.events
GROUP BY hour
```

```sql
SELECT
    toStartOfHour(event_time) AS hour,
    round(
        100 * countIf(event_type = 'subscribe') / nullIf(countIf(event_type = 'page_view'), 0),
        2
    ) AS conversion_pct,
    arrayStringConcat(
        arraySlice(groupUniqArray(referrer_domain), 1, 5),
        ', '
    ) AS top_referrer_domains
FROM raw.events
GROUP BY hour
```

```sql
SELECT
    toStartOfHour(event_time) AS hour,
    round(
        100 * countIf(event_type = 'subscribe')
            / nullIf(countIf(event_type = 'page_view'), 0),
        2
    ) AS conversion_pct,
    arrayStringConcat(
        arraySlice(groupUniqArray(referrer_domain), 1, 5),
        ', '
    ) AS top_referrer_domains
FROM raw.events
GROUP BY hour
```

For comparison: Prettier and pg_format's `--wrap-limit` default to 80, Black and sqlfmt to 88, rustfmt to 100.
The hand-written code in the local corpora peaks at 99–106 columns (`health.sql` has 104- and 106-column WHERE lines).
At 80, column definitions such as `sero_time DateTime64(3, 'UTC') DEFAULT inserted_at CODEC(Delta, ZSTD(1)),`
(77 columns plus indent) start to break.

**Recommendation: 100, configurable.**
That's what the authors already write, it matches rustfmt
(chum is Rust), and it leaves room for codecs and comments on a column line.
At 120 the second line above is 119 columns, too wide to read or to diff side by side.

### b. Long lists, and short lists

**`IN` with 12 literals.**
Fill (Prettier's rule for arrays of numbers or short literals):

```sql
DELETE FROM raw.events
WHERE event_type IN (
    'page_view', 'article_view', 'video_start', 'video_complete', 'scroll_25', 'scroll_50',
    'scroll_75', 'scroll_100', 'share', 'comment', 'subscribe', 'login'
)
```

One per line (Black, sqlfmt):

```sql
DELETE FROM raw.events
WHERE event_type IN (
    'page_view',
    'article_view',
    'video_start',
    'video_complete',
    'scroll_25',
    'scroll_50',
    'scroll_75',
    'scroll_100',
    'share',
    'comment',
    'subscribe',
    'login'
)
```

**Array literal**, fill vs one per line:

```sql
WITH
    [
        1024, 4096, 16384, 65536, 262144, 1048576, 4194304, 16777216, 67108864, 268435456,
        1073741824
    ] AS size_buckets_bytes
SELECT arrayMap(b -> formatReadableSize(b), size_buckets_bytes) AS labels
```

```sql
WITH
    [
        1024,
        4096,
        16384,
        65536,
        262144,
        1048576,
        4194304,
        16777216,
        67108864,
        268435456,
        1073741824
    ] AS size_buckets_bytes
SELECT arrayMap(b -> formatReadableSize(b), size_buckets_bytes) AS labels
```

**`SETTINGS` with 6 entries.**
One line (clickhouse-format, 177 columns here), fill, or one per line:

```sql
CREATE TABLE landing.events
(
    event_time DateTime64(3, 'UTC')
)
ENGINE = MergeTree
ORDER BY event_time
SETTINGS ttl_only_drop_parts = 1, merge_with_ttl_timeout = 900, min_bytes_for_wide_part = 33554432, index_granularity = 8192, storage_policy = 'hot_cold', allow_nullable_key = 1
```

```sql
CREATE TABLE landing.events
(
    event_time DateTime64(3, 'UTC')
)
ENGINE = MergeTree
ORDER BY event_time
SETTINGS
    ttl_only_drop_parts = 1, merge_with_ttl_timeout = 900, min_bytes_for_wide_part = 33554432,
    index_granularity = 8192, storage_policy = 'hot_cold', allow_nullable_key = 1
```

```sql
CREATE TABLE landing.events
(
    event_time DateTime64(3, 'UTC')
)
ENGINE = MergeTree
ORDER BY event_time
SETTINGS
    ttl_only_drop_parts = 1,
    merge_with_ttl_timeout = 900,
    min_bytes_for_wide_part = 33554432,
    index_granularity = 8192,
    storage_policy = 'hot_cold',
    allow_nullable_key = 1
```

**`ORDER BY (…)`** short, then too long for one line:

```sql
CREATE TABLE analytics.teaser_clicks
(
    site LowCardinality(String),
    sero_time DateTime('UTC'),
    product_feature LowCardinality(String),
    content_id String
)
ENGINE = SummingMergeTree
ORDER BY (site, sero_time, product_feature, content_id)
```

```sql
CREATE TABLE analytics.teaser_clicks_by_variant
(
    site LowCardinality(String),
    sero_time DateTime('UTC'),
    content_id String
)
ENGINE = SummingMergeTree
ORDER BY (
    site,
    toStartOfHour(sero_time),
    payload_product_feature,
    payload_data_data_teaser_id,
    payload_data_data_variant_id
)
```

**Recommendation:** every list stays on one line if it fits.
If it doesn't, one item per line, except a list whose items are all literals
(`IN (…)`, array literals, `quantiles(0.5, 0.9, …)` parameters), which fills.
One item per line gives one-line diffs, and a `SETTINGS` key or `ORDER BY` key is something you review one at a time;
`tally`'s hand-written SETTINGS already looks like that.
A column of numbers or status strings is the exception that Prettier also makes:
one per line adds height and tells the reader nothing.

**Does a short SELECT list stay on one line?**
This is the one list where the corpora disagree with themselves.
`health.sql` writes `SELECT database, table, name, reason` but also expands a 98-column list,
and the DESIGN.md view expands a list that would fit in 100.
Always split (clickhouse-format), fit (sqlfmt), or keep the author's choice (Prettier's object-literal rule):

```sql
SELECT
    database,
    table,
    name,
    reason
FROM system.detached_parts
```

```sql
SELECT database, table, name, reason
FROM system.detached_parts
```

Under "fit", the DESIGN.md view collapses to
`SELECT toStartOfHour(event_time) AS hour, site, count() AS views, uniqState(visitor_id) AS visitors` (99 columns).
Under "author's choice", a line break anywhere inside the input's list keeps it expanded,
and a list written on one line stays there while it fits.

**Recommendation: always split a SELECT list with more than one item,** like a column list.
A select list is a schema (in a view, literally), adding a column is a one-line diff,
and the collapse of the view above is the wrong result for chairlift's main input, migrations.
"Author's choice" is the runner-up, but it makes output depend on input layout,
which is what an opinionated formatter is meant to remove.
GROUP BY, ORDER BY and the rest follow the fit rule (`GROUP BY hour, site`).

### c. Leading vs trailing AND/OR, and commas

When a condition doesn't fit: hanging
(the condition starts on the WHERE line), WHERE alone (sqlfmt), or trailing operators.

```sql
SELECT count()
FROM raw.events
WHERE event_date >= today() - 7
    AND event_type IN ('page_view', 'article_view')
    AND site != ''
    AND NOT is_bot
```

```sql
SELECT count()
FROM raw.events
WHERE
    event_date >= today() - 7
    AND event_type IN ('page_view', 'article_view')
    AND site != ''
    AND NOT is_bot
```

```sql
SELECT count()
FROM raw.events
WHERE event_date >= today() - 7 AND
    event_type IN ('page_view', 'article_view') AND
    site != '' AND
    NOT is_bot
```

**Recommendation: leading operators, hanging (the first).**
Leading `AND` lets you comment out or Jinja-guard any condition but the first, and `WHERE true` takes care of the first.
Hanging keeps `WHERE true` on one line as the DESIGN.md example writes it;
sqlfmt's form would put `true` alone on a line.
The same rule applies to `ON`, `PREWHERE`, `HAVING`, and to OR inside parentheses:
`AND (a OR b)` stays on one line if it fits, otherwise the parenthesised group breaks the same way, one level deeper.

Commas, trailing vs leading:

```sql
SELECT
    toStartOfHour(event_time) AS hour,
    site,
    count() AS views
FROM raw.events
GROUP BY hour, site
```

```sql
SELECT
    toStartOfHour(event_time) AS hour
    , site
    , count() AS views
FROM raw.events
GROUP BY hour, site
```

**Recommendation: trailing.** clickhouse-format, `SHOW CREATE`, the ClickHouse docs, sqlfmt,
Black and Prettier all use them, and so does every file in the corpora.
A comment can pin a comma where the author put it (see f), so a few leading commas can survive in trailing-comma output.

### d. Column definitions: align or not

```sql
CREATE TABLE analytics.sessions
(
    session_id UUID,
    site LowCardinality(String) COMMENT 'Publication slug',
    started_at DateTime64(3, 'UTC') CODEC(Delta, ZSTD(1)) COMMENT 'First event of the session',
    duration_ms UInt32 CODEC(T64, ZSTD(1)),
    is_logged_in Bool DEFAULT false
)
ENGINE = MergeTree
ORDER BY (site, started_at)
```

```sql
CREATE TABLE analytics.sessions
(
    session_id   UUID,
    site         LowCardinality(String) COMMENT 'Publication slug',
    started_at   DateTime64(3, 'UTC') CODEC(Delta, ZSTD(1)) COMMENT 'First event of the session',
    duration_ms  UInt32 CODEC(T64, ZSTD(1)),
    is_logged_in Bool DEFAULT false
)
ENGINE = MergeTree
ORDER BY (site, started_at)
```

```sql
CREATE TABLE analytics.sessions
(
    session_id   UUID,
    site         LowCardinality(String)                                COMMENT 'Publication slug',
    started_at   DateTime64(3, 'UTC')    CODEC(Delta, ZSTD(1))         COMMENT 'First event of the session',
    duration_ms  UInt32                  CODEC(T64, ZSTD(1)),
    is_logged_in Bool                    DEFAULT false
)
ENGINE = MergeTree
ORDER BY (site, started_at)
```

**Recommendation: no alignment.**
Adding `payload_product_feature` to an aligned table rewrites every line, which is why Black, rustfmt,
Prettier and `SHOW CREATE` don't align.
The corpora show the problem already happening: chum's test schema and the workshop's `schema.sql` align types,
and the workshop's `product_feature LowCardinality(String)` already breaks its own alignment.
The third option also runs past 100 columns as soon as one codec or default is long.

### e. Blank lines

The corpora group short statements without blank lines
(a role and its grants) and use two blank lines as section breaks between queries (`health.sql`).
Strict (at most one anywhere, always one between statements) vs lenient
(keep the author's 0–2 between statements, but require one next to any multi-line statement;
at most one inside a statement):

```sql
-- The writer role: inserts into landing.events and nothing else.

CREATE ROLE events_writer;

GRANT INSERT ON landing.events TO events_writer;

GRANT events_writer TO events_consumer;

ALTER USER events_consumer DEFAULT ROLE ALL;
```

```sql
-- The writer role: inserts into landing.events and nothing else.

CREATE ROLE events_writer;
GRANT INSERT ON landing.events TO events_writer;
GRANT events_writer TO events_consumer;
ALTER USER events_consumer DEFAULT ROLE ALL;
```

**Recommendation: lenient.**
Between statements, keep 0, 1 or 2 blank lines as written
(more collapse to 2, like Black's top level), but force one before and after a statement that spans several lines.
Inside a statement, keep at most one, and only between list items or before a clause
(the blank line before the commented column in DESIGN.md survives).
Drop blank lines right after `(` or before `)`.
A header comment keeps the 0 or 1 blank line the author gave it: with none, the comment belongs to the first statement;
with one, it is the file's header.
The file ends with exactly one newline and starts with no blank line.

### f. Comments

Own-line comments attach to the next node and take its indent; a trailing comment stays on its line,
two spaces after the code (the corpora's habit, and Black's).
A comment between `)` and `ENGINE` attaches to `ENGINE` at column 0.

```sql
CREATE TABLE analytics.page_views_1h
(
    -- Start of the hour, in UTC.
    hour DateTime('UTC'),
    site LowCardinality(String),  -- the publication's slug
    views UInt64
)
-- Partial aggregates merge on read; query with sum and uniqMerge.
ENGINE = AggregatingMergeTree
ORDER BY (site, hour)
```

A line comment ends a line, so a group that contains one can't stay on one line, even if it would fit:

```sql
SELECT count()
FROM raw.events
ORDER BY (
    site,  -- most queries filter on one site
    event_time
)
```

Comma after vs before a comment.
Moving a comma across a comment changes the token order, so the comment pins it.
Written `site,  -- slug` it stays put.
Written in leading style with the comment before the comma, the comma has to stay at the start of the next line,
even in trailing-comma output:

```sql
SELECT
    toStartOfHour(event_time) AS hour,
    site  -- the publication's slug
    , count() AS views
FROM raw.events
GROUP BY hour, site
```

That is ugly but correct, and it is rare in practice; the alternative is reordering tokens, which is ruled out.

A comment inside a condition that has to break goes on its own line before the operand it describes;
a trailing comment ends its operand's line:

```sql
SELECT count()
FROM raw.events
WHERE event_date >= today() - 7
    -- Bots inflate page views by about a third.
    AND NOT is_bot
    AND site != ''  -- empty for events from the test harness
```

A `/* */` comment inside a line is an atom like any other token and doesn't force a break.
A comment is never reflowed or re-wrapped, but an own-line comment block is re-indented as a whole
(keeping the relative indent of its lines, which sqlfmt strips).

### g. Jinja

Block tags at the enclosing SQL indent, at column 0, or at the enclosing indent with the contents one level deeper:

```sql
SELECT count()
FROM raw.events
WHERE true
    AND event_type = 'page_view'
    {% if this.cutoff is defined -%}
    AND event_time >= {{ this.cutoff | sql_datetime }}
    {% endif -%}
```

```sql
SELECT count()
FROM raw.events
WHERE true
    AND event_type = 'page_view'
{% if this.cutoff is defined -%}
    AND event_time >= {{ this.cutoff | sql_datetime }}
{% endif -%}
```

```sql
SELECT count()
FROM raw.events
WHERE true
    AND event_type = 'page_view'
    {% if this.cutoff is defined -%}
        AND event_time >= {{ this.cutoff | sql_datetime }}
    {% endif -%}
```

sqlfmt's documented rules (its changelog:
"jinja block tags can no longer be indented farther than any of their contents",
and since 0.18 "the contents of jinja blocks are now indented
if the block wraps onto multiple rows") put the tags at the depth of the enclosing clause keyword,
which in DESIGN.md's test was column 0, the second form.

**Recommendation: the first form, tags at the indent of the SQL they wrap, contents not indented further.**
The SQL keeps the indentation it has when rendered with or without the block,
and the `AND` lines stay aligned whether or not they are guarded.
A block tag on its own line acts like an own-line comment: the enclosing group must break.
A tag between statements goes to column 0, as in chum's `{% if env.tier == "prod" %}` template.
`{{ … }}` is an atom: its contents, including spacing inside the braces, are left as written,
and `events_{{ suffix }}` is one identifier.
A `{{ }}` inside a string literal is part of the string.
A `{% for %}` that emits list items stays at the list's indent:

```sql
SELECT
    site,
    {% for e in this.event_types %}
    countIf(event_type = '{{ e }}') AS {{ e }}_count,
    {% endfor %}
    count() AS total
FROM raw.events
GROUP BY site
```

### h. Nesting

A nested SELECT always expands, and its parentheses hug: `(` ends the line that opens it,
`)` starts a line at that line's indent.
That replaces clickhouse-format's lone `FROM` / `(`:

```sql
SELECT
    site,
    avg(views) AS avg_daily_views
FROM (
    SELECT
        site,
        toDate(event_time) AS day,
        count() AS views
    FROM raw.events
    GROUP BY site, day
) AS daily
GROUP BY site
```

`IN (subquery)` the same way:

```sql
SELECT count()
FROM raw.events
WHERE visitor_id IN (
    SELECT visitor_id
    FROM raw.subscriptions
    WHERE status = 'active'
)
```

**CTEs.**
The `WITH` list follows the list rule, so a CTE is indented under `WITH` (sqlfmt), or the first one hugs `WITH`:

```sql
WITH
    daily AS (
        SELECT
            site,
            toDate(event_time) AS day
        FROM raw.events
    )
SELECT
    site,
    count() AS days
FROM daily
GROUP BY site
```

```sql
WITH daily AS (
    SELECT
        site,
        toDate(event_time) AS day
    FROM raw.events
)
SELECT
    site,
    count() AS days
FROM daily
GROUP BY site
```

**Recommendation: the second** (hug when `WITH` has a single CTE, otherwise one per line indented):
one less level of indentation for the common case, and it reads like the FROM-subquery form.

**`CREATE MATERIALIZED VIEW … AS SELECT`:** `AS` ends the header line and `SELECT` starts at column 0,
as every file in the corpora writes it, so the view's clauses line up as a plain query's do.
If the header doesn't fit, it breaks before `TO`, indented.

```sql
CREATE MATERIALIZED VIEW analytics.events_daily_mv TO analytics.events_daily AS
SELECT
    toDate(event_time) AS day,
    event_name,
    count() AS cnt
FROM raw.events
GROUP BY day, event_name
```

**CASE:** one line if it fits; otherwise `CASE` stays on its line, each `WHEN … THEN` and `ELSE` one level in,
`END` back at CASE's line indent:

```sql
SELECT
    CASE
        WHEN duration_ms < 10000 THEN 'bounce'
        WHEN duration_ms < 60000 THEN 'skim'
        WHEN duration_ms < 300000 THEN 'read'
        ELSE 'deep read'
    END AS engagement,
    count() AS sessions
FROM analytics.sessions
GROUP BY engagement
```

**Lambdas:** a lambda is one argument and stays whole on its line when it fits, as here;
a body that doesn't fit breaks after `->`, one level in:

```sql
SELECT
    arrayFilter(
        (url, ts) -> ts >= session_start AND NOT startsWith(url, 'https://internal.example.com/'),
        urls,
        timestamps
    ) AS external_urls
FROM analytics.sessions
```

**JOIN with a multi-condition ON:** one line if it fits, else `ON` on the next line indented,
with leading `AND` at the same indent:

```sql
SELECT
    o.order_id,
    p.price
FROM raw.orders AS o
ASOF LEFT JOIN dim.prices AS p
    ON p.sku = o.sku
    AND p.currency = o.currency
    AND o.ordered_at >= p.valid_from
```

**INSERT … SELECT:** the column list on the INSERT line (fit rule), the SELECT at column 0:

```sql
INSERT INTO analytics.events_daily (day, event_name, cnt)
SELECT
    toDate(event_time) AS day,
    event_name,
    count() AS cnt
FROM raw.events
WHERE event_time < '2026-01-01 00:00:00'
GROUP BY day, event_name
```

**ALTER TABLE:** the commands are a list.
One command stays on the ALTER line if it fits (as the workshop writes them); several go one per line, indented:

```sql
ALTER TABLE raw.events ADD COLUMN IF NOT EXISTS element String AFTER duration_ms;

ALTER TABLE analytics.page_views_1h
    ADD COLUMN referrer LowCardinality(String) DEFAULT '' AFTER site,
    MODIFY TTL hour + INTERVAL 180 DAY;
```

**GRANT:** one line if it fits; otherwise break before `ON`, `TO` and `WITH GRANT OPTION`, indented:

```sql
GRANT SELECT ON analytics.page_views_1h TO reader;

GRANT SELECT, INSERT, ALTER UPDATE, ALTER DELETE
    ON analytics.page_views_1h_with_a_long_name
    TO analytics_writer, backfill_job, maintenance
    WITH GRANT OPTION;
```

## 3. ClickHouse-specific constructs

```sql
CREATE TABLE analytics.page_views ON CLUSTER '{cluster}'
(
    event_time DateTime64(3, 'UTC') CODEC(Delta, ZSTD(1)),
    visitor_id String COMMENT 'Anonymous, 16 hex digits',
    INDEX visitor_bf visitor_id TYPE bloom_filter(0.01) GRANULARITY 4,
    PROJECTION by_visitor (
        SELECT *
        ORDER BY visitor_id
    )
)
ENGINE = ReplicatedMergeTree
ORDER BY tuple()
SETTINGS index_granularity = 8192
COMMENT 'One row per page view'
```

```sql
CREATE TABLE analytics.visits
(
    site LowCardinality(String),
    event_time DateTime,
    visitor_id String
)
ENGINE = MergeTree
PRIMARY KEY (site, toStartOfHour(event_time))
ORDER BY (site, toStartOfHour(event_time), cityHash64(visitor_id))
SAMPLE BY cityHash64(visitor_id)
TTL event_time + INTERVAL 13 MONTH DELETE
SETTINGS ttl_only_drop_parts = 1
```

```sql
SELECT
    site,
    tag,
    count() AS views
FROM raw.articles
ARRAY JOIN tags AS tag
PREWHERE event_date = today()
WHERE event_type = 'page_view'
GROUP BY site, tag WITH TOTALS
ORDER BY views DESC
LIMIT 3 BY site
LIMIT 100
SETTINGS max_threads = 8
FORMAT JSONEachRow
```

```sql
SELECT
    * EXCEPT (raw_payload),
    status::UInt16 AS status_code,
    {limit:UInt32} AS row_limit
FROM raw.requests
WHERE event_time >= {since:DateTime} - INTERVAL 1 DAY
```

- **`SETTINGS` after `TTL`, and every storage clause:** one clause per line, in the author's order; never reordered,
  so `PRIMARY KEY` before or after `ORDER BY` stays where it is.
  SETTINGS follows the list rule (b).
- **`ORDER BY tuple()`:** an ordinary expression, one line.
- **`SAMPLE BY`, `PARTITION BY`, `TTL`:** own line each.
  A TTL with several rules (`TTL d + INTERVAL 1 MONTH TO VOLUME 'cold', d + INTERVAL 1 YEAR DELETE`) is a list: fit,
  or one rule per line.
- **`PROJECTION name (SELECT …)`:** the nested SELECT expands and hugs,
  like a subquery. clickhouse-format puts `(` alone, as for the column list; hugging keeps the projection to fewer,
  shallower lines.
- **`INDEX … TYPE … GRANULARITY`:** one line, like a column; breaks only if it doesn't fit,
  before `TYPE` and `GRANULARITY`.
- **`ON CLUSTER`:** stays on the header line (`CREATE TABLE x ON CLUSTER c`, `GRANT ON CLUSTER c …`).
- **`FORMAT`:** its own last clause, after SETTINGS.
  In `INSERT … FORMAT X` everything after the name is data, verbatim.
- **`LIMIT n BY`:** its own clause line, before the plain `LIMIT`.
- **`WITH TOTALS` / `WITH ROLLUP` / `WITH CUBE`:** on the GROUP BY line when it fits; if the keys split,
  on its own line at clause indent, after the keys.
- **`PREWHERE`, `ARRAY JOIN` / `LEFT ARRAY JOIN`:** clause lines like WHERE and JOIN.
- **`::` casts, `INTERVAL 1 DAY`, `{name:Type}`:** atoms, never broken inside,
  no spaces added around `::` or inside the braces.
- **`* EXCEPT (a)`, `* REPLACE (…)`, `COLUMNS('re')`:** one select item; the parenthesised list follows the list rule.
- **`COMMENT 'x'`:** stays at the end of its column line, a string literal is never split,
  and a long one may run past the width.
  The table's `COMMENT` is its own last clause, as clickhouse-format prints it.
- **`true`, `false`, `NULL`:** literals, not keywords; their case stays as written
  (clickhouse-format prints `true` too).
- **`;`:** at the end of the statement's last line, not on its own (unless the last line ends in a `--` comment).

## 4. The DESIGN.md example as chairlift should print it

```sql
-- Hourly page views per site. The blank line after this header is
-- intentional.

CREATE TABLE analytics.page_views_1h
(
    hour DateTime('UTC') CODEC(Delta, ZSTD(1)),
    site LowCardinality(String),
    views UInt64,

    -- A comment inside a column list must survive.
    visitors AggregateFunction(uniq, String)
)
ENGINE = AggregatingMergeTree
PARTITION BY toStartOfDay(hour)
ORDER BY (site, hour)
TTL hour + INTERVAL 90 DAY
SETTINGS ttl_only_drop_parts = 1;

CREATE MATERIALIZED VIEW analytics.page_views_1h_mv TO analytics.page_views_1h AS
SELECT
    toStartOfHour(event_time) AS hour,
    site,
    count() AS views,
    uniqState(visitor_id) AS visitors
FROM raw.events
WHERE true
    AND event_type = 'page_view'
    {% if this.cutoff is defined -%}
    AND event_time >= {{ this.cutoff | sql_datetime }}
    {% endif -%}
GROUP BY hour, site;

GRANT SELECT ON analytics.page_views_1h TO {{ this.reader_role | sql_ident }};
```

The changes from the input are keyword case only: `create table`, `create materialized view`, `as`, `select`, `from`,
`where`, `and` and `group by` become upper case, and `true` stays lower case.
Every line break, indent and blank line in the input already matches the proposal,
which is a good sign that the proposal describes how the authors already write.

## Questions for the project owner

1. Width 100, with a `--line-width` option (or none at all)?
2. Long lists: fill only all-literal lists, one per line for everything else?
3. SELECT lists: always split when more than one item, even short ones in ad-hoc query files like `health.sql`?
4. Hanging `WHERE first` + leading `AND`, and trailing commas?
5. No column alignment, accepting churn in the files that align today?
6. Lenient blank lines (0–2 between statements, forced around multi-line statements)?
7. Jinja block tags at the enclosing SQL indent rather than sqlfmt's column 0?
8. Single CTE hugging `WITH`, and subqueries/projections hugging their `(`
   while the column list keeps `(` on its own line?
