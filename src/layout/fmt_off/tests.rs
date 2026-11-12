use super::Marker;
use super::marker;
use super::regions;
use crate::Options;
use crate::lexer::Mode;
use crate::lexer::TokenKind;
use crate::lexer::tokenize;

fn markers(source: &str, mode: Mode) -> Vec<Option<Marker>> {
    tokenize(source, mode)
        .iter()
        .filter(|t| t.kind() != TokenKind::Whitespace)
        .map(|t| marker(t, source, mode))
        .collect()
}

#[test]
fn markers_are_spelled_as_sqlfmt_spells_them() {
    let one = |source: &str, mode| markers(source, mode)[0];
    for off in [
        "-- fmt: off",
        "--fmt:off",
        "-- fmt:off",
        "--fmt: off",
        "-- FMT: Off",
        "-- fmt: off  \t",
        "# fmt: off",
    ] {
        assert_eq!(one(off, Mode::Sql), Some(Marker::Off), "{off:?}");
    }
    assert_eq!(one("-- fmt: on", Mode::Sql), Some(Marker::On));
    for not in [
        "--  fmt: off",
        "-- fmt:  off",
        "-- fmt: off because",
        "-- fmt: of",
        "--- fmt: off",
        "-- fmt off",
        "// fmt: off",
        "/* fmt: off */",
        "'-- fmt: off'",
        "{# fmt: off #}",
    ] {
        assert_eq!(one(not, Mode::Sql), None, "{not:?}");
    }
    for off in ["{# fmt: off #}", "{#fmt:off#}", "{#-  fmt: OFF\n -#}"] {
        assert_eq!(one(off, Mode::Jinja), Some(Marker::Off), "{off:?}");
    }
    assert_eq!(one("{# fmt: on #}", Mode::Jinja), Some(Marker::On));
    assert_eq!(one("{# fmt: off, please #}", Mode::Jinja), None);
}

#[test]
fn regions_are_whole_lines_and_nest() {
    let texts = |source: &str| -> Vec<String> {
        let tokens = tokenize(source, Mode::Jinja);
        regions(source, &tokens, Mode::Jinja)
            .into_iter()
            .map(|(a, b)| source[tokens[a].start()..tokens[b].end()].to_owned())
            .collect()
    };
    assert_eq!(
        texts("a\n  b -- fmt: off\nc\n-- fmt: on\nd"),
        ["b -- fmt: off\nc\n-- fmt: on"]
    );
    assert_eq!(texts("a\n-- fmt: off\nb  \n"), ["-- fmt: off\nb"]);
    assert_eq!(texts("a -- fmt: on\nb"), Vec::<String>::new());
    // Regions with only whitespace between them are one.
    assert_eq!(
        texts("-- fmt: off\na\n-- fmt: on\n\n-- fmt: off\nb\n-- fmt: on\nc\n-- fmt: off\nd"),
        [
            "-- fmt: off\na\n-- fmt: on\n\n-- fmt: off\nb\n-- fmt: on",
            "-- fmt: off\nd"
        ]
    );
    assert_eq!(
        texts("-- fmt: off\n-- fmt: off\n-- fmt: on\na\n-- fmt: on\nb"),
        ["-- fmt: off\n-- fmt: off\n-- fmt: on\na\n-- fmt: on"]
    );
    assert_eq!(
        texts("a {# fmt: off #} b {# fmt: on #} c\nd"),
        ["a {# fmt: off #} b {# fmt: on #} c"]
    );
    assert_eq!(
        texts("-- fmt: off\na\n{# fmt: on #} {# fmt: off #}\nb\n-- fmt: on\nc"),
        ["-- fmt: off\na\n{# fmt: on #} {# fmt: off #}\nb\n-- fmt: on"]
    );
}

/// Formats `source`, checking that formatting again changes nothing.
fn fmt_in(source: &str, jinja: bool, width: u16) -> String {
    let options = Options::default().with_jinja(jinja).with_line_width(width);
    let once = crate::format(source, &options).unwrap_or_else(|e| panic!("{source:?}: {e}"));
    let twice = crate::format(&once, &options).unwrap_or_else(|e| panic!("{once:?}: {e}"));
    assert_eq!(twice, once, "not idempotent on {source:?}");
    once
}

/// Checks that `source` formats to `expected`, and `expected` to itself.
fn check_in(source: &str, expected: &str, jinja: bool, width: u16) {
    let expected = expected.strip_prefix('\n').unwrap_or(expected);
    assert_eq!(fmt_in(source, jinja, width), expected, "from {source:?}");
    assert_eq!(fmt_in(expected, jinja, width), expected);
}

fn check(source: &str, expected: &str) {
    check_in(source, expected, false, 100);
}

fn check_jinja(source: &str, expected: &str) {
    check_in(source, expected, true, 100);
}

#[test]
fn a_region_between_statements_keeps_its_lines() {
    check(
        "select 1;\n\n-- fmt: off\nselect   a,b   from   t;\n  select x  -- c\n-- fmt: on\n\nselect 2",
        "
SELECT 1;

-- fmt: off
select   a,b   from   t;
  select x  -- c
-- fmt: on

SELECT 2
",
    );
}

#[test]
fn a_region_in_a_select_list_keeps_its_alignment() {
    check(
        "select a,   b,\n-- fmt: off\n    cc  , ddd,\n    e   , f,\n-- fmt: on\n g from t where x=1",
        "
SELECT
    a,
    b,
-- fmt: off
    cc  , ddd,
    e   , f,
-- fmt: on
    g
FROM t
WHERE x = 1
",
    );
}

#[test]
fn a_region_in_a_case_keeps_its_alignment() {
    check(
        "select case\n  -- fmt: off\n  when a = 1   then   'x'\n  when a = 22  then   'yy'\n  \
         -- fmt: on\n  else 'z' end as v from t",
        "
SELECT
    CASE
  -- fmt: off
  when a = 1   then   'x'
  when a = 22  then   'yy'
  -- fmt: on
        ELSE 'z'
    END AS v
FROM t
",
    );
}

#[test]
fn a_region_in_an_in_list_keeps_its_rows() {
    check(
        "select a from t where x in (\n  1, 2,\n  -- fmt: off\n  3,   4,\n  5,   6,\n  -- fmt: on\n  7)",
        "
SELECT a
FROM t
WHERE x IN (
    1,
    2,
  -- fmt: off
  3,   4,
  5,   6,
  -- fmt: on
    7
)
",
    );
}

#[test]
fn a_region_in_a_values_list_keeps_its_table() {
    check(
        "select * from values('a UInt8, b String',\n    -- fmt: off\n    (1,   'one'),\n    \
         (22,  'twenty-two'),\n    -- fmt: on\n    (3, 'three'))",
        "
SELECT *
FROM values(
    'a UInt8, b String',
    -- fmt: off
    (1,   'one'),
    (22,  'twenty-two'),
    -- fmt: on
    (3, 'three')
)
",
    );
}

#[test]
fn a_region_across_a_statement_boundary_keeps_both_parts() {
    check(
        "select 1;\nselect a,\n-- fmt: off\n  b  from t;\nselect   c\n  from u\n-- fmt: on\n; select 2",
        "
SELECT 1;

SELECT
    a,
-- fmt: off
  b  from t;
select   c
  from u
-- fmt: on
;

SELECT 2
",
    );
}

/// A marker that trails code keeps that code's line too, as in sqlfmt's
/// example.
#[test]
fn a_trailing_marker_keeps_its_whole_line() {
    check(
        "-- fmt: off\nselect 1, 2, 3,       4, 5, 6,       7, 8, 9\n-- fmt: on\nfrom my_table\n\
         where something is true -- fmt: off\ngroup by 1, 2, 3,        4, 5, 6,       7, 8, 9\n",
        "
-- fmt: off
select 1, 2, 3,       4, 5, 6,       7, 8, 9
-- fmt: on
FROM my_table
where something is true -- fmt: off
group by 1, 2, 3,        4, 5, 6,       7, 8, 9
",
    );
    check(
        "select a,\n     b -- fmt: off\n   ,  c\n   -- fmt: on\n , d from t",
        "
SELECT
    a,
     b -- fmt: off
   ,  c
   -- fmt: on
    , d
FROM t
",
    );
    check(
        "select f(a, -- fmt: off\n  b,c) -- fmt: on\nfrom t",
        "
select f(a, -- fmt: off
  b,c) -- fmt: on
FROM t
",
    );
}

#[test]
fn an_unmatched_marker() {
    // An off without an on keeps the rest of the text, later statements
    // included; the text still ends with one newline.
    check(
        "select a,\n-- fmt: off\nb  ,  c\nfrom t;\n\n\n\n\nselect   1   \n\n",
        "
SELECT
    a,
-- fmt: off
b  ,  c
from t;




select   1
",
    );
    // An on without an off is a comment.
    check(
        "select a -- fmt: on\n, b from t",
        "
SELECT
    a  -- fmt: on
    , b
FROM t
",
    );
}

/// Markers nest, as in sqlfmt: an off inside a region needs an on of its
/// own.
#[test]
fn a_repeated_off_needs_its_own_on() {
    check(
        "select a,\n-- fmt: off\nb  ,c,\n-- fmt: off\nd  ,e,\n-- fmt: on\nf  ,  g,\n-- fmt: on\nh  , i from t",
        "
SELECT
    a,
-- fmt: off
b  ,c,
-- fmt: off
d  ,e,
-- fmt: on
f  ,  g,
-- fmt: on
    h,
    i
FROM t
",
    );
}

#[test]
fn marker_text_in_a_string_or_block_comment_is_no_marker() {
    check(
        "select '-- fmt: off', /* -- fmt: off */ a,b from t",
        "
SELECT
    '-- fmt: off', /* -- fmt: off */
    a,
    b
FROM t
",
    );
    check(
        "select 'a\n-- fmt: off\n', b from t",
        "
SELECT
    'a\n-- fmt: off\n',
    b
FROM t
",
    );
    // A marker after `FORMAT CSV` is part of the data.
    check(
        "insert into t format CSV\n-- fmt: off\n1,2\n\nselect   1",
        "
INSERT INTO t FORMAT CSV
-- fmt: off
1,2

SELECT 1
",
    );
}

/// The blank lines next to a region are kept as anywhere else in a
/// statement: one at most.
#[test]
fn the_lines_around_a_region_are_laid_out() {
    check(
        "select a,\n\n\n\n-- fmt: off\nb,\n-- fmt: on\n\n\n\nc  from t",
        "
SELECT
    a,

-- fmt: off
b,
-- fmt: on

    c
FROM t
",
    );
    // Blank lines and trailing whitespace inside a region stay.
    check(
        "select\n-- fmt: off\n\n\nb   , c  \n\n\n-- fmt: on\nfrom t",
        "SELECT\n-- fmt: off\n\n\nb   , c  \n\n\n-- fmt: on\nFROM t\n",
    );
    // A line of the region wider than the line width does not move what
    // follows it.
    check_in(
        "select a,\n-- fmt: off\nlong_name_one, long_name_two,\n-- fmt: on\nf(x, y) from t",
        "
SELECT
    a,
-- fmt: off
long_name_one, long_name_two,
-- fmt: on
    f(x, y)
FROM t
",
        false,
        20,
    );
}

/// A statement kept as written keeps the region's trailing whitespace,
/// which it otherwise drops.
#[test]
fn a_region_in_a_statement_kept_as_written() {
    check(
        "system   flush logs;  \n-- fmt: off\nsystem   flush  logs;   \n-- fmt: on\n",
        "
system   flush logs;
-- fmt: off
system   flush  logs;   \n-- fmt: on
",
    );
}

/// Indentation before the first token of a text is not kept, there or
/// anywhere else.
#[test]
fn a_region_at_the_start_of_a_text_starts_at_column_0() {
    check(
        "   -- fmt: off\n  select   1",
        "-- fmt: off\n  select   1\n",
    );
}

#[test]
fn jinja_comments_are_markers_in_templates() {
    check_jinja(
        "select a,\n{# fmt: off #}\n  b  ,   c,\n{# fmt: on #}\n d from t",
        "
SELECT
    a,
{# fmt: off #}
  b  ,   c,
{# fmt: on #}
    d
FROM t
",
    );
    // A region can start and end in the middle of a line; it keeps the
    // whole of both lines.
    check_jinja(
        "select a, {# fmt: off #} b  ,  c\n, d {# fmt: on #} , e\n, f from t",
        "
select a, {# fmt: off #} b  ,  c
, d {# fmt: on #} , e
    ,
    f
FROM t
",
    );
    // In plain SQL a Jinja comment is no marker.
    check(
        "select a,\n{# fmt: off #}\n  b  ,   c\n from t",
        "SELECT\n    a,\n    {# fmt: off #}\n      b  ,   c\n     from t\n",
    );
}

#[test]
fn regions_and_block_tags() {
    // Inside a block: the tags around the region are laid out.
    check_jinja(
        "select a,\n{% if x %}\n-- fmt: off\n  b  ,   c,\n-- fmt: on\n{% endif %}\n d from t",
        "
SELECT
    a,
    {% if x %}
-- fmt: off
  b  ,   c,
-- fmt: on
    {% endif %}
    d
FROM t
",
    );
    // Around a block: its tags are kept as written.
    check_jinja(
        "select a,\n-- fmt: off\n{% if x %}\n  b  ,   c,\n   {% endif %}\n-- fmt: on\n d from t",
        "
SELECT
    a,
-- fmt: off
{% if x %}
  b  ,   c,
   {% endif %}
-- fmt: on
    d
FROM t
",
    );
    // An opener in the region, its closer after it: the closer lines up
    // with the opener as written.
    check_jinja(
        "select a,\n-- fmt: off\n  {% if x %}\n  b  ,   c,\n-- fmt: on\n      {% endif %}\n d from t",
        "
SELECT
    a,
-- fmt: off
  {% if x %}
  b  ,   c,
-- fmt: on
  {% endif %}
    d
FROM t
",
    );
    check_jinja(
        "select a from t\nwhere x = 1\n  {% if y %}\n  -- fmt: off\n  and   y = 2\n  {% endif %}\n  \
         -- fmt: on\n and z = 3",
        "
SELECT a
FROM t
WHERE x = 1
    {% if y %}
  -- fmt: off
  and   y = 2
  {% endif %}
  -- fmt: on
    AND z = 3
",
    );
    // After a dbt header.
    check_jinja(
        "{{ config(x=1) }}\n-- fmt: off\nselect   1\n",
        "{{ config(x=1) }}\n-- fmt: off\nselect   1\n",
    );
}
