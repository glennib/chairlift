use super::*;

fn ok(input: &str, output: &str) {
    if let Err(e) = verify(input, output, Mode::Sql, &Facts::new()) {
        panic!("{input:?} -> {output:?}: {e}");
    }
}

fn fails(input: &str, output: &str, mode: Mode, facts: &Facts) -> VerifyError {
    match verify(input, output, mode, facts) {
        Ok(()) => panic!("{input:?} -> {output:?} passed"),
        Err(e) => e,
    }
}

fn sql_fails(input: &str, output: &str) -> VerifyError {
    fails(input, output, Mode::Sql, &Facts::new())
}

/// The index of the token with `text` in `source`'s split.
fn token_index(source: &str, text: &str) -> usize {
    split(source, Mode::Sql)
        .tokens()
        .iter()
        .position(|t| t.text(source) == text)
        .unwrap()
}

#[test]
fn layout_may_change() {
    ok("SELECT a,b FROM t", "SELECT\n    a,\n    b\nFROM t\n");
    ok("SELECT 1 -- c  \r\n", "SELECT 1 -- c\n");
    ok("SELECT 1 -- c\u{2003}\u{a0}", "SELECT 1 -- c\n");
    ok("\u{feff}SELECT\u{a0}1", "SELECT 1");
    ok("", "\n");
}

#[test]
fn tokens_may_not_change() {
    let e = sql_fails("SELECT a, b", "SELECT a b");
    assert_eq!(
        e.detail,
        "token 3 is Comma \",\" in the input and BareWord \"b\" in the output"
    );
    assert_eq!((e.input.line, e.input.column), (1, 9));
    sql_fails("SELECT a", "SELECT a, b");
    sql_fails("SELECT a, b", "SELECT a");
    sql_fails("SELECT 'a b'", "SELECT 'a  b'");
    sql_fails("SELECT 'a\r\nb'", "SELECT 'a\nb'");
    sql_fails("SELECT $$a\r\n$$", "SELECT $$a\n$$");
    sql_fails("SELECT `a`", "SELECT \"a\"");
    sql_fails("SELECT 1.0", "SELECT 1.00");
}

#[test]
fn comments_keep_their_text() {
    sql_fails("SELECT 1 -- c", "SELECT 1 --  c");
    sql_fails("SELECT 1 /* a\r\n b */", "SELECT 1 /* a\n b */");
    sql_fails("SELECT 1 /* c  */", "SELECT 1 /* c */");
    sql_fails("SELECT 1 -- c", "SELECT 1");
    // `# ` without its space is an error token, not a comment.
    sql_fails("SELECT 1 # \n", "SELECT 1 #\n");
}

#[test]
fn a_printer_that_glues_minus_signs_is_caught() {
    let e = sql_fails("SELECT 1 - -1, 2", "SELECT 1 --1, 2");
    assert_eq!(
        e.detail,
        "token 3 is Minus \"-\" in the input and LineComment \"--1, 2\" in the output"
    );
}

#[test]
fn a_line_comment_that_loses_its_newline_is_caught() {
    sql_fails("SELECT 1 -- c\n, 2", "SELECT 1 -- c , 2");
}

#[test]
fn case_changes_only_where_recorded() {
    let input = "select true, select";
    sql_fails(input, "SELECT true, select");
    let mut facts = Facts::new();
    facts.recase(0);
    assert!(facts.is_recased(0) && !facts.is_recased(2));
    verify(input, "SELECT true, select", Mode::Sql, &facts).unwrap();
    // Only into its canonical case.
    fails(input, "SeLeCt true, select", Mode::Sql, &facts);
    // Only the recorded word.
    fails(input, "SELECT true, SELECT", Mode::Sql, &facts);
    fails(input, "SELECT TRUE, select", Mode::Sql, &facts);
    // Only its case.
    fails(input, "SELEKT true, select", Mode::Sql, &facts);
}

/// A keyword's canonical case is upper case, a literal word's lower case,
/// `NULL`'s upper case; a word that is neither keeps its case.
#[test]
fn recased_words_take_their_canonical_case() {
    let input = "SELECT TRUE, Inf, null, Infinity, nAn, toDate, x FROM t";
    let mut facts = Facts::new();
    for word in [
        "SELECT", "TRUE", "Inf", "null", "Infinity", "nAn", "toDate", "x", "FROM",
    ] {
        facts.recase(token_index(input, word));
    }
    verify(
        input,
        "SELECT true, inf, NULL, infinity, nan, toDate, x FROM t",
        Mode::Sql,
        &facts,
    )
    .unwrap();
    // Unchanged is always fine.
    verify(input, input, Mode::Sql, &facts).unwrap();
    fails(
        input,
        "select TRUE, Inf, null, Infinity, nAn, toDate, x FROM t",
        Mode::Sql,
        &facts,
    );
    fails(
        input,
        "SELECT TRUE, INF, null, Infinity, nAn, toDate, x FROM t",
        Mode::Sql,
        &facts,
    );
    fails(
        input,
        "SELECT TRUE, Inf, Null, Infinity, nAn, toDate, x FROM t",
        Mode::Sql,
        &facts,
    );
    // Not keywords: no case is canonical.
    fails(
        input,
        "SELECT TRUE, Inf, null, Infinity, nAn, TODATE, x FROM t",
        Mode::Sql,
        &facts,
    );
    fails(
        input,
        "SELECT TRUE, Inf, null, Infinity, nAn, toDate, X FROM t",
        Mode::Sql,
        &facts,
    );
}

#[test]
fn recording_a_token_that_is_not_a_bare_word_allows_nothing() {
    let input = "SELECT 'a', `b`";
    let mut facts = Facts::new();
    facts.recase(token_index(input, "'a'"));
    facts.recase(token_index(input, "`b`"));
    fails(input, "SELECT 'A', `b`", Mode::Sql, &facts);
    fails(input, "SELECT 'a', `B`", Mode::Sql, &facts);
}

#[test]
fn recased_tokens_far_into_a_file() {
    let mut facts = Facts::new();
    facts.recase(1000);
    assert!(facts.is_recased(1000));
    assert!(!facts.is_recased(999) && !facts.is_recased(1001) && !facts.is_recased(1 << 20));
}

#[test]
fn verbatim_regions_keep_their_whitespace() {
    let input = "SELECT [3,4,5]::Array(Int64)";
    let output = "SELECT [3, 4, 5]::Array(Int64)";
    ok(input, output);
    let mut facts = Facts::new();
    facts.keep_verbatim(token_index(input, "["), token_index(input, "]"));
    let e = fails(input, output, Mode::Sql, &facts);
    assert_eq!(e.input.column, 11);
    assert_eq!(e.output.column, 11);
    verify(
        input,
        "SELECT\n    [3,4,5]::Array(Int64)",
        Mode::Sql,
        &facts,
    )
    .unwrap();
}

#[test]
fn a_verbatim_region_must_be_named_by_its_tokens() {
    let input = "SELECT [3, 4]";
    let mut facts = Facts::new();
    facts.keep_verbatim(token_index(input, " "), token_index(input, "]"));
    fails(input, input, Mode::Sql, &facts);
}

#[test]
fn insert_data_keeps_every_byte() {
    sql_fails(
        "INSERT INTO t VALUES (1),  (2);",
        "INSERT INTO t VALUES (1), (2);",
    );
    sql_fails(
        "INSERT INTO t VALUES (1),\r\n(2);",
        "INSERT INTO t VALUES (1),\n(2);",
    );
    sql_fails(
        "INSERT INTO t FORMAT CSV\n1, 2  \n3,4\n\nSELECT 1;",
        "INSERT INTO t FORMAT CSV\n1, 2\n3,4\n\nSELECT 1;",
    );
    ok(
        "INSERT INTO t FORMAT CSV\n1,2\n\n\n\nSELECT 1;",
        "INSERT INTO t FORMAT CSV\n1,2\n\nSELECT 1;",
    );
    ok("INSERT INTO t VALUES (1)\n  ;", "INSERT INTO t VALUES (1);");
}

#[test]
fn moving_the_end_of_insert_data_is_caught() {
    // Without the blank line, the next statement becomes part of the data.
    sql_fails(
        "INSERT INTO t FORMAT CSV\n1,2\n\nSELECT 1;",
        "INSERT INTO t FORMAT CSV\n1,2\nSELECT 1;",
    );
    // The line break after the format name is not data, but a second one is.
    ok(
        "INSERT INTO t FORMAT CSV   \r\n1,2",
        "INSERT INTO t FORMAT CSV\n1,2",
    );
    sql_fails(
        "INSERT INTO t FORMAT CSV\n1,2",
        "INSERT INTO t FORMAT CSV\n\n1,2",
    );
}

#[test]
fn jinja_tags_keep_their_bytes() {
    let none = Facts::new();
    fails("SELECT {{ x }}", "SELECT {{x}}", Mode::Jinja, &none);
    fails(
        "{% if a %}SELECT 1{% endif %}",
        "{%if a%}SELECT 1{% endif %}",
        Mode::Jinja,
        &none,
    );
    fails("SELECT 1 {# c #}", "SELECT 1", Mode::Jinja, &none);
    fails(
        "-- {{ x\r\n }}\nSELECT 1",
        "-- {{ x\n }}\nSELECT 1",
        Mode::Jinja,
        &none,
    );
    verify(
        "SELECT {{ x }} ,1",
        "SELECT {{ x }}\n, 1",
        Mode::Jinja,
        &none,
    )
    .unwrap();
}

/// sqlfmt's case c10: moving `{%- if %}` up to the comment's line made the
/// rendered SQL put the condition inside the comment.
#[test]
fn whitespace_stripped_by_a_marker_is_checked_as_rendered() {
    let input = "SELECT 1\nWHERE x = 1 -- note\n{%- if y %}\n  AND y = 2\n{% endif %}";
    let output = "SELECT 1\nWHERE x = 1  -- note\n    {%- if y %} AND y = 2\n{% endif %}";
    // The tokens are the same; only the rendering differs.
    verify(input, output, Mode::Sql, &Facts::new()).unwrap();
    let e = fails(input, output, Mode::Jinja, &Facts::new());
    assert!(e.detail.starts_with("rendered,"), "{e}");
    verify(
        input,
        "SELECT 1\nWHERE x = 1 -- note\n    {%- if y %}\n    AND y = 2\n{% endif %}",
        Mode::Jinja,
        &Facts::new(),
    )
    .unwrap();
}

#[test]
fn rendering_strips_whitespace_after_a_marker() {
    let none = Facts::new();
    // `-%}` strips the line break either way, so only the space before the
    // tag keeps `-` and `-1` apart once rendered.
    verify(
        "SELECT 1 - {% if a -%}\n-1",
        "SELECT 1 - {% if a -%}-1",
        Mode::Jinja,
        &none,
    )
    .unwrap();
    let e = fails(
        "SELECT 1 - {% if a -%}-1",
        "SELECT 1 -{% if a -%}-1",
        Mode::Jinja,
        &none,
    );
    assert!(e.detail.contains("LineComment"), "{e}");
    verify(
        "SELECT {{- x -}} ,1",
        "SELECT {{- x -}},\n1",
        Mode::Jinja,
        &none,
    )
    .unwrap();
}

#[test]
fn rendering_as_the_renderer_joins_text() {
    assert_eq!(rendered("a  {{- x -}}  b").0, "a{{- x -}}b");
    assert_eq!(rendered("a \n{%- if c %} b {% endif -%}\n c").0, "a b c");
    assert_eq!(rendered("a {# c #} b {#- c -#} d").0, "a  bd");
    // Tags inside comments and strings strip too: the renderer does not
    // know SQL.
    assert_eq!(rendered("-- c {%- if x -%}  \nd").0, "-- cd");
    assert_eq!(rendered("' {{- x }} '").0, "'{{- x }} '");
    // A raw block renders as its text, which its inner markers strip.
    assert_eq!(rendered("a {%- raw -%} x {%- endraw -%} b").0, "axb");
    assert_eq!(
        rendered("a {% raw %} {{ x }} {% endraw %} b").0,
        "a  {{ x }}  b"
    );
    assert_eq!(rendered("a {% raw %}").0, "a ");
    assert_eq!(rendered("no tags").0, "no tags");
}

/// What an expression renders as is unknown, so whitespace next to it may
/// change but not appear or disappear.
#[test]
fn whitespace_next_to_an_expression_stays_or_stays_absent() {
    let none = Facts::new();
    let jinja_ok = |input: &str, output: &str| {
        verify(input, output, Mode::Jinja, &none).unwrap_or_else(|e| panic!("{input:?}: {e}"));
    };
    let e = fails("SELECT - {{ x }}", "SELECT -{{ x }}", Mode::Jinja, &none);
    assert!(e.detail.contains("glued"), "{e}");
    fails("SELECT {{ x }},1", "SELECT {{ x }} ,1", Mode::Jinja, &none);
    fails(
        "SELECT 1 {% raw %}x{% endraw %} 1",
        "SELECT 1{% raw %}x{% endraw %} 1",
        Mode::Jinja,
        &none,
    );
    jinja_ok(
        "SELECT 1 {% raw %}-{% endraw %} 1",
        "SELECT 1 {% raw %}-{% endraw %}\n\n1",
    );
    jinja_ok("SELECT - {{ x }}", "SELECT -\n  {{ x }}\n");
    jinja_ok("SELECT {{ x }},1", "SELECT {{ x }},1\n");
    // Once rendered, the block tag is gone and `-` meets the expression.
    fails(
        "SELECT - {% if a %}{{ x }}",
        "SELECT -{% if a %}{{ x }}",
        Mode::Jinja,
        &none,
    );
    // `-}}` strips the space either way.
    jinja_ok("SELECT {{- x -}} ,1", "SELECT {{- x -}},\n1");
    // A word with a tag glued into it is a word.
    jinja_ok("SELECT -events_{{ s }}", "SELECT -events_{{ s }}\n");
    // The ends of the text are not neighbouring tokens.
    jinja_ok("{{ x }}", "{{ x }}\n");
}

/// A raw block renders as its text, which joins the SQL around it.
#[test]
fn raw_blocks_render_as_their_text() {
    let none = Facts::new();
    let e = fails(
        "SELECT {% raw %}-{% endraw %} -1",
        "SELECT {% raw %}-{% endraw %}-1",
        Mode::Jinja,
        &none,
    );
    assert!(e.detail.contains("LineComment"), "{e}");
    fails(
        "SELECT {% raw %}-{% endraw %}\n.5",
        "SELECT {% raw %}-{% endraw %}\n. 5",
        Mode::Jinja,
        &none,
    );
    let e = fails(
        "SELECT {% raw %}'{% endraw %}a  \n  b{% raw %}'{% endraw %}",
        "SELECT {% raw %}'{% endraw %}a\n  b{% raw %}'{% endraw %}",
        Mode::Jinja,
        &none,
    );
    assert!(e.detail.contains("StringLiteral"), "{e}");
}

/// Removing block and comment tags can join template tokens into one literal
/// of the rendered SQL, whose text then holds the whitespace between them.
#[test]
fn whitespace_inside_a_rendered_literal_is_its_text() {
    let none = Facts::new();
    let heredoc = "SELECT {% if a %}$x$ \n\n\n {% endif %}$x$";
    let e = fails(
        heredoc,
        "SELECT {% if a %}$x$\n\n {% endif %}$x$",
        Mode::Jinja,
        &none,
    );
    assert!(e.detail.contains("HereDoc"), "{e}");
    let comment = "SELECT 1 /{# c #}* x  \n *{# c #}/ + 2";
    let e = fails(
        comment,
        "SELECT 1 /{# c #}* x\n *{# c #}/ + 2",
        Mode::Jinja,
        &none,
    );
    assert!(e.detail.contains("BlockComment"), "{e}");
}

/// A tag with markers inside a line comment strips the comment's own trailing
/// whitespace and the line break after it, in input and output alike.
#[test]
fn a_marker_inside_a_comment_strips_its_trailing_whitespace() {
    let input = "-- c{%--%}  \nSELECT 1";
    verify(input, "-- c{%--%}\nSELECT 1\n", Mode::Jinja, &Facts::new()).unwrap();
}

#[test]
fn error_display() {
    let e = sql_fails("SELECT\n  a, b", "SELECT\n  a b");
    assert_eq!(
        e.to_string(),
        "the output differs from the input at input 2:4, output 2:5: \
         token 3 is Comma \",\" in the input and BareWord \"b\" in the output"
    );
}
