use crate::Options;

/// Formats `source` at `width`, checking that formatting again changes
/// nothing.
fn fmt_in(source: &str, jinja: bool, width: u16) -> String {
    let options = Options::default().with_jinja(jinja).with_line_width(width);
    let once = crate::format(source, &options).unwrap_or_else(|e| panic!("{source:?}: {e}"));
    let twice = crate::format(&once, &options).unwrap_or_else(|e| panic!("{once:?}: {e}"));
    assert_eq!(twice, once, "not idempotent on {source:?}");
    once
}

fn fmt(source: &str) -> String {
    fmt_in(source, false, 100)
}

/// Checks that `source` formats to `expected`, and `expected` to itself.
fn check(source: &str, expected: &str) {
    check_at(source, expected, 100);
}

fn check_at(source: &str, expected: &str, width: u16) {
    let expected = expected.strip_prefix('\n').unwrap_or(expected);
    assert_eq!(fmt_in(source, false, width), expected, "from {source:?}");
    assert_eq!(fmt_in(expected, false, width), expected);
}

#[test]
fn the_header_stays_on_one_line_and_the_query_starts_a_line() {
    check(
        "insert into analytics.events_daily (day, event_name, cnt) select toDate(event_time) as day, event_name, count() as cnt from raw.events group by day, event_name",
        "
INSERT INTO analytics.events_daily (day, event_name, cnt)
SELECT
    toDate(event_time) AS day,
    event_name,
    count() AS cnt
FROM raw.events
GROUP BY day, event_name
",
    );
    check(
        "insert into table function file('out.csv', 'CSV') partition by k select k from t",
        "
INSERT INTO TABLE FUNCTION file('out.csv', 'CSV') PARTITION BY k
SELECT k
FROM t
",
    );
    check(
        "insert into t (select 1 union all select 2)",
        "
INSERT INTO t (
    SELECT 1
    UNION ALL
    SELECT 2
)
",
    );
}

/// A column list that does not fit goes one column per line, and what
/// follows it stays on the line of its `)`.
#[test]
fn a_long_column_list_breaks_one_column_per_line() {
    check(
        "insert into analytics.page_views_by_site_and_hour (site, hour, page_views, unique_visitors, sessions) values (1, 2, 3, 4, 5)",
        "
INSERT INTO analytics.page_views_by_site_and_hour (
    site,
    hour,
    page_views,
    unique_visitors,
    sessions
) VALUES (1, 2, 3, 4, 5)
",
    );
}

/// The clauses of the header go on lines of their own when it does not
/// fit.
#[test]
fn header_clauses_break_at_the_statement_indentation() {
    check(
        "insert into t settings async_insert = 1 values (1)",
        "INSERT INTO t SETTINGS async_insert = 1 VALUES (1)\n",
    );
    check_at(
        "insert into t (a, b) from infile 'data.csv.gz' compression 'gzip' settings max_insert_threads = 4 format CSV",
        "
INSERT INTO t (a, b)
FROM INFILE 'data.csv.gz' COMPRESSION 'gzip'
SETTINGS max_insert_threads = 4
FORMAT CSV
",
        60,
    );
}

/// The data keeps its bytes and the lines it is on; the rows of `VALUES`
/// on the header line follow it after a space.
#[test]
fn the_data_stays_as_written() {
    check(
        "insert   into t  values   (1, 'a'),   (2,'b') ;",
        "INSERT INTO t VALUES (1, 'a'),   (2,'b') ;\n",
    );
    check(
        "insert into t values\n  (1, 'a'),\n  (2, 'b');",
        "\nINSERT INTO t VALUES\n  (1, 'a'),\n  (2, 'b');\n",
    );
    check(
        "insert into t format CSV\n1,  'a'\n2,'b'\n\nselect 1",
        "INSERT INTO t FORMAT CSV\n1,  'a'\n2,'b'\n\nSELECT 1\n",
    );
    // Data in another format starts after the spaces ClickHouse skips.
    check(
        "insert into t format TSV   x\ty\n",
        "INSERT INTO t FORMAT TSV x\ty\n",
    );
    check(
        "insert into t format TSV  \r\n  x\ty\n",
        "INSERT INTO t FORMAT TSV\n  x\ty\n",
    );
    // A line comment at the end of the rows keeps the `;` on its line.
    check(
        "insert into t values (1) -- one\n;",
        "INSERT INTO t VALUES (1) -- one\n;\n",
    );
    // `FORMAT Values` takes rows up to the `;`, as `VALUES` does.
    check(
        "insert into t format Values    (1)",
        "INSERT INTO t FORMAT Values (1)\n",
    );
}

/// A `WITH` clause goes before the header, and a `FORMAT` after the query
/// on a line of its own.
#[test]
fn with_before_and_format_after_the_query() {
    check(
        "with x as (select 1) insert into t select * from x",
        "
WITH x AS (
    SELECT 1
)
INSERT INTO t
SELECT *
FROM x
",
    );
    check(
        "insert into t select * from input('a UInt8') format CSV\n1\n2\n",
        "
INSERT INTO t
SELECT *
FROM input('a UInt8')
FORMAT CSV
1
2
",
    );
}

/// Comments and block tags between the parts stay where they belong.
#[test]
fn comments_and_tags_between_the_parts() {
    check(
        "insert into t -- the target\n(a) select 1",
        "
INSERT INTO t  -- the target
    (a)
SELECT 1
",
    );
    assert_eq!(
        fmt_in(
            "insert into t {% if x %} settings a = 1 {% endif %} select 1",
            true,
            100
        ),
        "INSERT INTO t\n{% if x %}\nSETTINGS a = 1\n{% endif %}\nSELECT 1\n"
    );
}

/// Statements around one with data spanning lines are set apart by a blank
/// line, as around any statement that spans lines. What follows the rows
/// of `VALUES` on their line stays as written.
#[test]
fn statements_around_data_keep_their_lines() {
    assert_eq!(
        fmt("insert into t values (1);\ninsert into t values (2); -- two\n"),
        "INSERT INTO t VALUES (1);\nINSERT INTO t VALUES (2); -- two\n"
    );
    assert_eq!(
        fmt("insert into t format CSV\n1\n\ninsert into t format CSV\n2\n"),
        "INSERT INTO t FORMAT CSV\n1\n\nINSERT INTO t FORMAT CSV\n2\n"
    );
}

/// A query alone in brackets hugs them; a comment that ends the target's
/// line puts the column list one level in.
#[test]
fn a_bracketed_query_hugs_and_a_comment_indents_the_columns() {
    check(
        "insert into t (select 1);\ninsert into t (a) (select 1)",
        "
INSERT INTO t (
    SELECT 1
);

INSERT INTO t (a) (
    SELECT 1
)
",
    );
    check(
        "insert into t -- c\n(a, b) values (1, 2)",
        "
INSERT INTO t  -- c
    (a, b)
VALUES (1, 2)
",
    );
}
