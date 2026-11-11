use super::Decision;
use super::Kept;
use crate::Options;
use crate::format_with_statements;

/// Formats `source`, checking that formatting again changes nothing.
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

fn jinja(source: &str) -> String {
    fmt_in(source, true, 100)
}

/// What the layout decided for each statement of `source`.
fn decisions(source: &str, jinja: bool) -> Vec<Decision> {
    let options = Options::default().with_jinja(jinja);
    let (_, statements) =
        format_with_statements(source, &options).unwrap_or_else(|e| panic!("{source:?}: {e}"));
    statements.iter().map(|s| s.decision).collect()
}

#[test]
fn comments_between_list_items() {
    // A blank line around a comment between items stays, also before a
    // comma the comment pins.
    assert_eq!(
        fmt("select a,\n\n-- c\n\nb,\n\nc -- t\n\n, d\nfrom t"),
        "SELECT\n    a,\n\n    -- c\n\n    b,\n\n    c  -- t\n\n    , d\nFROM t\n"
    );
    assert_eq!(
        fmt("select a, -- first\n  b /* inline */, c\nfrom t"),
        "SELECT\n    a,  -- first\n    b /* inline */,\n    c\nFROM t\n"
    );
    // An own-line comment goes with the item after it, a blank line before
    // it stays, and comments on consecutive lines keep their relative
    // indentation.
    assert_eq!(
        fmt("select a,\n\n  -- one\n    -- two\n  b from t"),
        "SELECT\n    a,\n\n    -- one\n      -- two\n    b\nFROM t\n"
    );
    // A blank line after `(` goes, one between items stays.
    assert_eq!(
        fmt("select f(\n\n  a,\n\n\n  -- c\n  b)"),
        "SELECT\n    f(\n        a,\n\n        -- c\n        b\n    )\n"
    );
}

#[test]
fn comments_after_a_clause_keyword_go_with_its_body() {
    assert_eq!(
        fmt(
            "select\n-- k\na\nfrom\n-- t\nt\njoin\n-- u\nu on x\nwhere\n-- w\na = 1\n\
             group by\n-- g\na\nlimit -- l\n10"
        ),
        "SELECT\n    -- k\n    a\nFROM\n    -- t\n    t\nJOIN\n    -- u\n    u\n    ON x\nWHERE\n    -- w\n    \
         a = 1\nGROUP BY\n    -- g\n    a\nLIMIT  -- l\n    10\n"
    );
    assert_eq!(
        fmt("create table t (a Int8) engine =\n-- e\nMergeTree order by\n-- o\na"),
        "CREATE TABLE t\n(\n    a Int8\n)\nENGINE =\n    -- e\n    MergeTree\nORDER BY\n    -- o\n    a\n"
    );
}

/// Text read as written stays so in statements kept as written, found for
/// each by a search rather than a scan of every statement.
#[test]
fn literals_read_as_text_in_kept_statements_keep_their_bytes() {
    let source = "insert into t values (1);\nselect [1,  2]::String;\n".repeat(3)
        + "{% if x %}select '{% if y %}', [1,  2]::String;{% endif %}\n";
    let out = jinja(&source);
    assert_eq!(out.matches("[1,  2]::String").count(), 4, "{out}");
}

/// In a template, `INSERT … FORMAT` data whose extent the rendering can
/// change is kept as written, and so is what follows it; a later `FORMAT`
/// of another statement does not count.
#[test]
fn template_data_is_kept_and_other_formats_are_not() {
    let source = "{% if x %}\nselect 1;\n{% endif %}\ninsert into t select 1;\n\
                  select 2 format Null;\nselect 3;";
    assert_eq!(
        decisions(source, true),
        [
            Decision::Formatted,
            Decision::Kept(Kept::NotParsed),
            Decision::Formatted,
            Decision::Formatted
        ]
    );
    let source = "{% if x %}\nselect 1;\n{% endif %}\ninsert into t format CSV\n1\n\nselect 2;";
    assert_eq!(
        decisions(source, true),
        [
            Decision::Formatted,
            Decision::Kept(Kept::NotParsed),
            Decision::Kept(Kept::TemplateData)
        ]
    );
    // A block tag inside the `INSERT` can leave out the `;` that ends it.
    let source = "insert into t select 1 {% if x %};{% endif %}\nselect 2 format CSV;";
    assert_eq!(
        decisions(source, true)[1..],
        [Decision::Kept(Kept::TemplateData)]
    );
}

#[test]
fn the_list_of_limit_by_follows_the_list_rule() {
    assert_eq!(
        fmt("select a from t limit 3 by a, b limit 5"),
        "SELECT a\nFROM t\nLIMIT 3 BY a, b\nLIMIT 5\n"
    );
    assert_eq!(
        fmt_in(
            "select a from t limit 3 by aaaaaaaaaa, bbbbbbbbbb",
            false,
            20
        ),
        "SELECT a\nFROM t\nLIMIT 3 BY\n    aaaaaaaaaa,\n    bbbbbbbbbb\n"
    );
}

#[test]
fn comments_between_clauses_and_before_the_semicolon() {
    assert_eq!(
        fmt("select 1 -- one\n-- before from\nfrom t /* t */\nwhere x -- x\n;"),
        "SELECT 1  -- one\n-- before from\nFROM t /* t */\nWHERE x  -- x\n;\n"
    );
}

#[test]
fn a_comment_inside_an_expression_breaks_its_group() {
    assert_eq!(
        fmt("select a + -- why\n b from t"),
        "SELECT\n    a\n        +  -- why\n        b\nFROM t\n"
    );
}

#[test]
fn a_comment_on_the_line_of_what_follows_stays_on_it() {
    // Before a comma, it pins the comma, as a line comment does.
    assert_eq!(
        fmt("select a\n/* c */ ,\n b"),
        "SELECT\n    a\n    /* c */, b\n"
    );
    // A Jinja comment keeps the whitespace after it, which once rendered
    // separates the expression before it from the comma.
    assert_eq!(
        jinja("select a, {{- c -}}\n\n{# c #} ,\nd\nfrom t\n"),
        "SELECT\n    a,\n    {{- c -}}\n\n    {# c #} , d\nFROM t\n"
    );
    // After an opening bracket, a comment that trails it is spaced from
    // what follows only.
    assert_eq!(
        fmt("select f( /* c */a), [ /* d */ 1 ], g(a /* e */\n)"),
        "SELECT\n    f(/* c */ a),\n    [/* d */ 1],\n    g(a /* e */)\n"
    );
    assert_eq!(
        fmt("select 1;\n/* a */\n/* b */ select 2"),
        "SELECT 1;\n/* a */\n/* b */ SELECT 2\n"
    );
    assert_eq!(jinja("{#\n#} {##} select 1"), "{#\n#}\n{##} SELECT 1\n");
    // After a comment on a line of its own, the next starts a line too.
    assert_eq!(fmt("/* a *//* b */select 1"), "/* a */\n/* b */ SELECT 1\n");
    assert_eq!(
        fmt("select 1;\n/* a */ -- b\nselect 2"),
        "SELECT 1;\n/* a */ -- b\nSELECT 2\n"
    );
}

#[test]
fn comments_at_the_start_and_end_of_the_file() {
    assert_eq!(
        fmt("\n\n-- header\n-- more\n\n\n\nselect 1; -- done\n\n-- tail\n"),
        "-- header\n-- more\n\nSELECT 1;  -- done\n\n-- tail\n"
    );
    assert_eq!(fmt("/* only */"), "/* only */\n");
}

#[test]
fn a_statement_on_several_lines_gets_blank_lines_also_past_comments() {
    assert_eq!(
        fmt("select 1;\n-- the next one\nselect a, b from t;\nselect 2;"),
        "SELECT 1;\n\n-- the next one\nSELECT\n    a,\n    b\nFROM t;\n\nSELECT 2;\n"
    );
}

#[test]
fn set_operations_are_flat() {
    assert_eq!(
        fmt("select 1 union all select 2 union distinct (select 3) settings a = 1 format JSON"),
        "SELECT 1\nUNION ALL\nSELECT 2\nUNION DISTINCT\n(\n    SELECT 3\n)\nSETTINGS a = 1\nFORMAT JSON\n"
    );
}

#[test]
fn several_ctes_go_one_per_line_under_with() {
    assert_eq!(
        fmt("with a as (select 1), b as (select 2) select * from a, b"),
        "WITH\n    a AS (\n        SELECT 1\n    ),\n    b AS (\n        SELECT 2\n    )\nSELECT *\nFROM a, b\n"
    );
    assert_eq!(
        fmt("with 1 as x, 2 as y select x + y"),
        "WITH 1 AS x, 2 AS y\nSELECT x + y\n"
    );
}

#[test]
fn group_by_modifiers_follow_the_keys() {
    assert_eq!(
        fmt("select count() from t group by grouping sets ((a, b), (a), ()) with totals"),
        "SELECT count()\nFROM t\nGROUP BY GROUPING SETS ((a, b), (a), ()) WITH TOTALS\n"
    );
    assert_eq!(
        fmt_in("select 1 group by aaaa, bbbb with rollup", false, 20),
        "SELECT 1\nGROUP BY\n    aaaa,\n    bbbb\nWITH ROLLUP\n"
    );
}

#[test]
fn joins_go_on_lines_of_their_own() {
    assert_eq!(
        fmt("select * from t1 join t2 on t1.a = t2.a and t1.b = t2.b left array join arr as x"),
        "SELECT *\nFROM t1\nJOIN t2 ON t1.a = t2.a AND t1.b = t2.b\nLEFT ARRAY JOIN arr AS x\n"
    );
    assert_eq!(
        fmt("select * from (t1 join t2 using (a)) cross join t3, t4"),
        "SELECT *\nFROM (\n    t1\n    JOIN t2 USING (a)\n)\nCROSS JOIN t3,\nt4\n"
    );
}

#[test]
fn clickhouse_clauses() {
    assert_eq!(
        fmt(
            "select n from numbers(10) order by n with fill from 1 to 20 step 2 interpolate (x as x + 1)"
        ),
        "SELECT n\nFROM numbers(10)\nORDER BY n WITH FILL FROM 1 TO 20 STEP 2\nINTERPOLATE (x AS x + 1)\n"
    );
    assert_eq!(
        fmt("select a from t final sample 1/10 offset 1/2 window w as (order by b) qualify x = 1"),
        "SELECT a\nFROM t FINAL SAMPLE 1 / 10 OFFSET 1 / 2\nWINDOW w AS (order by b)\nQUALIFY x = 1\n"
    );
    assert_eq!(
        fmt("select a from t offset 5 rows fetch first 10 rows only"),
        "SELECT a\nFROM t\nOFFSET 5 ROWS\nFETCH FIRST 10 ROWS ONLY\n"
    );
    assert_eq!(
        fmt("select x into outfile 'f.csv' format CSV"),
        "SELECT x\nINTO OUTFILE 'f.csv'\nFORMAT CSV\n"
    );
}

#[test]
fn or_inside_parentheses_hangs_one_level_deeper() {
    assert_eq!(
        fmt_in(
            "select 1 where (a = 1 or b = 2) and c = 3 and (dddd = 4 or eeee = 5)",
            false,
            25
        ),
        "SELECT 1\nWHERE (a = 1 OR b = 2)\n    AND c = 3\n    AND (dddd = 4\n        OR eeee = 5)\n"
    );
    // As the first operand, too: deeper than the `AND`s.
    assert_eq!(
        fmt_in("select 1 where (aaaa = 1 or bbbb = 2) and c = 3", false, 25),
        "SELECT 1\nWHERE (aaaa = 1\n        OR bbbb = 2)\n    AND c = 3\n"
    );
    // In a join's condition, which hangs under `ON`.
    assert_eq!(
        fmt_in(
            "select 1 from t join u on (aaaa = 1 or bbbb = 2) and c = 3",
            false,
            25
        ),
        "SELECT 1\nFROM t\nJOIN u\n    ON (aaaa = 1\n        OR bbbb = 2)\n    AND c = 3\n"
    );
}

/// After an operand whose last line starts with its closer (`)`, `END`),
/// the operator stays on that line.
#[test]
fn an_operator_stays_after_a_broken_operand() {
    assert_eq!(
        fmt("select 1 from t where (select count() from u) > 0"),
        "SELECT 1\nFROM t\nWHERE (\n    SELECT count()\n    FROM u\n) > 0\n"
    );
    assert_eq!(
        fmt_in(
            "select case when a = 1 then 'b' else 'c' end = 'x'",
            false,
            30
        ),
        "SELECT\n    CASE\n        WHEN a = 1 THEN 'b'\n        ELSE 'c'\n    END = 'x'\n"
    );
    assert_eq!(
        fmt_in("select f(aaaaaaaa, bbbbbbbb, cccccccc) + 1", false, 30),
        "SELECT\n    f(\n        aaaaaaaa,\n        bbbbbbbb,\n        cccccccc\n    ) + 1\n"
    );
    assert_eq!(
        fmt_in("select [aaaaaaaa, bbbbbbbb, cccccccc] = x", false, 30),
        "SELECT\n    [\n        aaaaaaaa,\n        bbbbbbbb,\n        cccccccc\n    ] = x\n"
    );
    // On the closer's line, more that cannot break may follow it.
    assert_eq!(
        fmt_in(
            "select (f(aaaaaaaa, bbbbbbbb, cccccccc) as x) > 0",
            false,
            30
        ),
        "SELECT\n    (f(\n        aaaaaaaa,\n        bbbbbbbb,\n        cccccccc\n    ) AS x) > 0\n"
    );
    // After a closer followed by more that can break, or one that ends a
    // line of other code, the operator breaks before it as anywhere else,
    // rather than hide in the middle of a line.
    assert_eq!(
        fmt_in(
            "select f(aaaaaaaa, bbbbbbbb, cccccccc) * g(1) + 1",
            false,
            30
        ),
        "SELECT\n    f(\n        aaaaaaaa,\n        bbbbbbbb,\n        cccccccc\n    ) * g(1)\n        + 1\n"
    );
    assert_eq!(
        fmt_in(
            "select ('aaaaaaaaaaaaaaaa' || 'bbbbbbbbbbbbbbbbbbbbbbbb' || 'c') = 'x'",
            false,
            30
        ),
        "SELECT\n    ('aaaaaaaaaaaaaaaa'\n        || 'bbbbbbbbbbbbbbbbbbbbbbbb'\n        || 'c')\n        = 'x'\n"
    );
    assert_eq!(
        fmt_in(
            "select aaaaaaaa + bbbbbbbb + e['cccccccc'] > 8 * f(dddd)",
            false,
            30
        ),
        "SELECT\n    aaaaaaaa + bbbbbbbb\n        + e['cccccccc']\n        > 8 * f(dddd)\n"
    );
    assert_eq!(
        fmt_in(
            "select (aaaaaaaa / bbbbbbbbbbbbbbbbbbb) / pow(cccccccccc, 2)",
            false,
            30
        ),
        "SELECT\n    (aaaaaaaa\n        / bbbbbbbbbbbbbbbbbbb)\n        / pow(cccccccccc, 2)\n"
    );
    // An operand too long for its line but without a place to break.
    assert_eq!(
        fmt_in("select aaaaaaaaaaaa.bbbbbbbbbbbbbbbbbbb + c", false, 30),
        "SELECT\n    aaaaaaaaaaaa.bbbbbbbbbbbbbbbbbbb\n        + c\n"
    );
    // An operand that fits, with the operator breaking before it.
    assert_eq!(
        fmt_in(
            "select aaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            false,
            30
        ),
        "SELECT\n    aaaaaaaaaaaa\n        + bbbbbbbbbbbbbbbbbbbbbbbbbbbb\n"
    );
    // A closer that touches a tag stays on the tag's line.
    assert_eq!(
        fmt_in("select f(aaaaaaaa, bbbbbbbb, {{ c }}) + 1", true, 30),
        "SELECT\n    f(\n        aaaaaaaa,\n        bbbbbbbb,\n        {{ c }})\n        + 1\n"
    );
    // On the closer's line, the operator needs room for the start of its
    // operand, which may break on its own.
    assert_eq!(
        fmt_in(
            "select f(aaaaaaaa, bbbbbbbb, cccccccc) = g(dddddddd, eeeeeeee, ffffffff)",
            false,
            30
        ),
        "SELECT\n    f(\n        aaaaaaaa,\n        bbbbbbbb,\n        cccccccc\n    ) = g(\n        dddddddd,\n        eeeeeeee,\n        ffffffff\n    )\n"
    );
    assert_eq!(
        fmt_in(
            "select f(aaaaaaaa, bbbbbbbb, cccccccc) = 'a string too long for the line'",
            false,
            30
        ),
        "SELECT\n    f(\n        aaaaaaaa,\n        bbbbbbbb,\n        cccccccc\n    )\n        = 'a string too long for the line'\n"
    );
}

/// The review's statements on operators after brackets, at several widths:
/// formatting is idempotent, and no line over the width has an operator
/// after a closer, where a line could have broken instead.
#[test]
fn operators_after_brackets_at_several_widths() {
    let statements = [
        "select a from t where (select count() from u) > 0",
        "select someFunctionWithALongName(first_argument_column, second_argument_column, third_argument) + 1 as x from t",
        "select some_very_long_array_column_name[1] + another_very_long_array_column_name[2] + yet_another_long_name[3] as s from t",
        "select (aaaaaaaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb) * (ccccccccccccccccccccccccccccccc - dddddddddddddddddddddddddd) as p from t",
        "select f(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbbbbbbbbbb, cccccccccccccccccccccccccccc) = g(dddddddddddddddddddddd, eeeeeeeeeeeeeeeeeeeeeeeeeeeeee) as eq from t",
        "select aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa[1] + bbbbbbbbbbbbbbbbbbbbbbbb as x from t",
        "select a from t where count() > 1 and (select max(x) from u where u.k = t.k and u.v > 100000) > (select min(y) from w)",
        "select a from t where (aaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbb + ccccccccccccccccccccccccccc) / 2 > ddddddddddddddddddddd",
        "select (toUnixTimestamp(finished_at_with_a_long_name) - toUnixTimestamp(started_at_with_a_long_name)) * (weight_column_name + offset_column_name) as weighted from t",
        "select (sumIf(duration_ms, event_type = 'inscreen') + sumIf(duration_ms, event_type = 'heartbeat_ping')) * (1 - bounce_rate_for_this_site) / 1000 as read_seconds from t",
        "select case when a = 1 then 'one' when a = 2 then 'two' else 'many' end = 'one' as c from t",
    ];
    let operators = ["+", "-", "*", "/", "=", ">", "<", "!="];
    for width in [40, 60, 80, 100] {
        for statement in statements {
            let out = fmt_in(statement, false, width);
            for line in out.lines() {
                if line.chars().count() <= usize::from(width) {
                    continue;
                }
                let after_closer = operators.iter().any(|op| {
                    line.contains(&format!(") {op} ")) || line.contains(&format!("] {op} "))
                });
                assert!(
                    !after_closer,
                    "an operator after a closer on a line over {width}:\n{line}\n--- in\n{out}"
                );
            }
        }
    }
}

/// A token that spans lines breaks every group around it; the operator
/// before it stays where it is.
#[test]
fn an_operator_stays_before_a_multi_line_token() {
    assert_eq!(
        fmt("select 1 where x = 'a\nb' and y = 1"),
        "SELECT 1\nWHERE x = 'a\nb'\n    AND y = 1\n"
    );
    assert_eq!(fmt("select 'a\nb' || 'c'"), "SELECT\n    'a\nb' || 'c'\n");
}

#[test]
fn between_never_breaks_at_its_and() {
    assert_eq!(
        fmt_in("select 1 where a between 1 and 2 and b = 3", false, 20),
        "SELECT 1\nWHERE a BETWEEN 1 AND 2\n    AND b = 3\n"
    );
}

#[test]
fn spacing_of_expressions() {
    // JSON paths.
    assert_eq!(
        fmt("select json.^a, json.a.:Int64 from t"),
        "SELECT\n    json.^a,\n    json.a.:Int64\nFROM t\n"
    );
    assert_eq!(
        fmt(
            "select -x, not y, f(1)(2), a.b, t.*, x::Int8, a[1], x.1, {p:UInt8}, @@v, - -1, {'k': 1}"
        ),
        "SELECT\n    -x,\n    NOT y,\n    f(1)(2),\n    a.b,\n    t.*,\n    x::Int8,\n    a[1],\n    x.1,\n    \
         {p:UInt8},\n    @@v,\n    - -1,\n    {'k': 1}\n"
    );
    assert_eq!(
        fmt("select cast(x as UInt8), extract(day from d), * except (a), columns('c') apply(sum)"),
        "SELECT\n    cast(x AS UInt8),\n    extract(DAY FROM d),\n    * EXCEPT (a),\n    \
         COLUMNS('c') APPLY(sum)\n"
    );
}

#[test]
fn literals_read_as_text_keep_their_bytes() {
    assert_eq!(
        fmt("select [1,2,  3]::Array(UInt8)"),
        "SELECT [1,2,  3]::Array(UInt8)\n"
    );
    // A Jinja expression can render as a literal: `[1,2]::String` is
    // '[1,2]' and `[1, 2]::String` '[1, 2]'.
    assert_eq!(
        jinja("select [{{ x }},2]::String, cast([{{ x }},1] as String), [a,1]::String"),
        "SELECT\n    [{{ x }},2]::String,\n    cast([{{ x }},1] AS String),\n    [a, 1]::String\n"
    );
    let source = "select [{{ x }},2]::String";
    let parse = crate::syntax::parse(source, crate::lexer::Mode::Jinja);
    assert_eq!(parse.facts().verbatim_regions(), [(2, 6)]);
}

#[test]
fn verbatim_items_and_clauses_keep_their_text() {
    // An item that does not parse.
    assert_eq!(
        fmt("select a, b c d, e from t"),
        "SELECT\n    a,\n    b c d,\n    e\nFROM t\n"
    );
}

// Jinja.

#[test]
fn block_tags_between_clauses_and_statements() {
    assert_eq!(
        jinja("select 1\n{% if x %}\nwhere a\n{% else %}\nwhere b\n{% endif %}\n"),
        "SELECT 1\n{% if x %}\nWHERE a\n{% else %}\nWHERE b\n{% endif %}\n"
    );
    // Between statements a tag goes to column 0.
    assert_eq!(
        jinja("  {% if x %}\n  select 1;\n  {% endif %}\nselect 2;"),
        "{% if x %}\nSELECT 1;\n{% endif %}\nSELECT 2;\n"
    );
}

#[test]
fn a_closer_aligns_with_its_opener() {
    assert_eq!(
        jinja("select a,\n{% if x %}\nb,\n{% endif %}\nc from t"),
        "SELECT\n    a,\n    {% if x %}\n    b,\n    {% endif %}\n    c\nFROM t\n"
    );
}

#[test]
fn jinja_shapes_the_layout_does_not_cover_are_kept_as_written() {
    // A block tag inside an expression.
    let source = "select a + {% if x %} b {% else %} c {% endif %} from t";
    assert_eq!(jinja(source), format!("{source}\n"));
    assert_eq!(decisions(source, true), [Decision::Kept(Kept::TagPlace)]);
    // A tag touching a word.
    let source = "select a,{% if x %} b {% endif %} from t";
    assert_eq!(decisions(source, true), [Decision::Kept(Kept::TagTouching)]);
    // A tag inside a string: every statement is kept.
    let source = "select '{% if x %}';\n{% endif %}\nselect 2;";
    assert_eq!(
        decisions(source, true),
        [
            Decision::Kept(Kept::HiddenTag),
            Decision::Kept(Kept::HiddenTag)
        ]
    );
    // Rendering joins tokens into one heredoc.
    let source = "select {% if a %}$x$ {% endif %}$x$";
    assert_eq!(
        decisions(source, true),
        [Decision::Kept(Kept::RenderJoined)]
    );
}

/// A line comment that the rendering can make run on into text the layout
/// moves leaves the whole text as it is: what it swallows could change in
/// renders the verifier does not check.
#[test]
fn a_line_comment_the_rendering_runs_on_leaves_the_text_alone() {
    use crate::CommentEnd;
    let options = Options::default().with_jinja(true);
    for (source, reason) in [
        // With `x` false, the input renders `select a -- c from t`.
        (
            "select a -- c\n{%- if x %}\n, b\n{% endif %} from t\n",
            CommentEnd::MarkerAfter,
        ),
        (
            "select\n    a\n    {%- for i in xs -%}\n    ,\n    c{{ i }} -- c\n    {%- endfor %}\nfrom t\n",
            CommentEnd::MarkerAfter,
        ),
        (
            "select 1; -- c\n{#- note #};select 2",
            CommentEnd::MarkerAfter,
        ),
        // A tag inside a comment can remove the line break after it.
        (
            "-- {% if x %}\nselect 1;\n{% endif %}\nselect 2;",
            CommentEnd::TagInside,
        ),
        ("{% x %}# {%- y %} \n1\n", CommentEnd::TagInside),
        // With `x` false, the input's comment swallows `c UInt8,`, which the
        // layout puts on a line of its own.
        (
            "create table t (\n    a UInt8,  -- note {{ v -}}\n    {% if x %}\n    b UInt8,\n    \
             {% else %} c UInt8,\n    {% endif %}\n    d UInt8\n)\nengine = MergeTree\norder by a",
            CommentEnd::MarkerInside,
        ),
        ("select 1 -- c {{ v -}}\n, 2", CommentEnd::MarkerInside),
        // Where the macro is called, the comment runs on into what follows.
        (
            "{% macro m() %}\nselect 1 -- c\n{%- endmacro %}\n{{ m() }} union all select 2",
            CommentEnd::MacroBody,
        ),
    ] {
        match crate::format(source, &options) {
            Err(crate::Error::Template(e)) => assert_eq!(e.reason, reason, "{source:?}"),
            other => panic!("{source:?}: {other:?}"),
        }
    }
    let Err(crate::Error::Template(e)) =
        crate::format("select a -- c\n{%- if x %} b\n{% endif %}\n", &options)
    else {
        panic!("formatted");
    };
    assert_eq!((e.position.line, e.position.column), (1, 10));
    // A line break the rendering keeps ends a comment as in SQL.
    assert_eq!(
        jinja("select a -- c\n{% if x %}\n, b\n{% endif %} from t\n"),
        "SELECT\n    a  -- c\n    {% if x %}\n    , b\n    {% endif %}\nFROM t\n"
    );
    // Where the render can only go on after block tags and Jinja comments
    // that end their line, the comment runs on into nothing but their line
    // break, before and after formatting.
    for source in [
        "select\n    a,\n    -- the b column\n    {%- if x %}\n    b,\n    {%- endif %}\n    c\nfrom t",
        "-- header\n{%- set d = 7 %}\nselect {{ d }}",
        "-- header\n\n{#- about m -#}\n{% macro m() -%}\nselect 1\n{%- endmacro %}\n\n{{ m() }}",
        "select 1 -- c {{ v -}}\n{% if x %}\n, 2\n{% endif %}\n",
        "select a -- c\n{%- if x %}\n",
    ] {
        jinja(source);
    }
}

#[test]
fn whitespace_next_to_a_strip_marker_may_move() {
    // `-%}` strips the whitespace after it whatever it is.
    assert_eq!(
        jinja("select 1\nwhere true\n  and a\n{% if x -%}\nand b\n{%- endif %}"),
        "SELECT 1\nWHERE true\n    AND a\n    {% if x -%}\n    AND b\n    {%- endif %}\n"
    );
}

#[test]
fn expressions_are_atoms_and_keep_their_adjacency() {
    assert_eq!(
        jinja("select {{ x }}, events_{{ s }}, f({{ a }})from {{ t }}"),
        "SELECT\n    {{ x }},\n    events_{{ s }},\n    f({{ a }})\nFROM {{ t }}\n"
    );
}

#[test]
fn statements_the_parser_does_not_know_are_kept() {
    assert_eq!(
        decisions(
            "select 1; create table t (a Int8) engine = Memory; drop table t; ;",
            false
        ),
        [
            Decision::Formatted,
            Decision::Formatted,
            Decision::Kept(Kept::NotParsed),
            Decision::Formatted
        ]
    );
}

#[test]
fn an_empty_statement_stays_on_the_line_before_unless_a_tag_comes_between() {
    assert_eq!(fmt("select 1;;\n;"), "SELECT 1;;\n;\n");
    assert_eq!(jinja("};{%-{}%} ;"), "};\n{%-{}%}\n;\n");
}

/// Whitespace kept as written before a statement does not make it span
/// lines.
#[test]
fn a_kept_line_break_before_a_statement_is_not_inside_it() {
    // `FORMAT` keeps the whitespace after its statement in a template.
    assert_eq!(
        jinja("FORMAT D;$%;{%-%}L;$%-"),
        "FORMAT D;\n$%;\n{%-%}L;\n$%-\n"
    );
}
