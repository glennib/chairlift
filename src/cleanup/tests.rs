use crate::Error;
use crate::Options;
use crate::format;
use crate::lexer::TokenKind;

/// Formats `source`, checking that formatting again changes nothing.
fn fmt_in(source: &str, jinja: bool) -> String {
    let options = Options::default().with_jinja(jinja);
    let once = format(source, &options).unwrap_or_else(|e| panic!("{source:?}: {e}"));
    let twice = format(&once, &options).unwrap_or_else(|e| panic!("{once:?}: {e}"));
    assert_eq!(twice, once, "not idempotent on {source:?}");
    once
}

fn fmt(source: &str) -> String {
    fmt_in(source, false)
}

#[test]
fn trailing_whitespace_and_crlf() {
    assert_eq!(fmt("SELECT 1, \t\r\n  2 \r\n"), "SELECT 1,\n  2\n");
    assert_eq!(fmt("SELECT 1\u{a0}\u{3000}\n"), "SELECT 1\n");
    // Whitespace inside a line is not trailing.
    assert_eq!(fmt("SELECT  1 ,\t2\n"), "SELECT  1 ,\t2\n");
    // A lone `\r` is not a line ending.
    assert_eq!(fmt("SELECT 1\r2\n"), "SELECT 1\r2\n");
}

#[test]
fn indentation_is_kept() {
    let source = "  SELECT\n        a,\n\tb\n  FROM t\n";
    assert_eq!(fmt(source), source);
}

#[test]
fn blank_lines_between_statements() {
    assert_eq!(fmt("SELECT 1;SELECT 2;"), "SELECT 1;SELECT 2;\n");
    assert_eq!(fmt("SELECT 1;\nSELECT 2;"), "SELECT 1;\nSELECT 2;\n");
    assert_eq!(fmt("SELECT 1;\n\nSELECT 2;"), "SELECT 1;\n\nSELECT 2;\n");
    assert_eq!(
        fmt("SELECT 1;\n\n\nSELECT 2;"),
        "SELECT 1;\n\n\nSELECT 2;\n"
    );
    assert_eq!(
        fmt("SELECT 1;\n \n\t\n  \n\n-- c\n\n\n\n\nSELECT 2;"),
        "SELECT 1;\n\n\n-- c\n\n\nSELECT 2;\n"
    );
}

#[test]
fn blank_lines_inside_a_statement() {
    assert_eq!(
        fmt("SELECT\n\n\n  1\n\n\nFROM t"),
        "SELECT\n\n  1\n\nFROM t\n"
    );
    assert_eq!(
        fmt("SELECT f(\n\n  1,\n\n  2\n\n)"),
        "SELECT f(\n  1,\n\n  2\n)\n"
    );
    // Only round brackets.
    assert_eq!(fmt("SELECT [\n\n1\n\n]"), "SELECT [\n\n1\n\n]\n");
    // A comment after `(` does not count as the bracket.
    assert_eq!(
        fmt("SELECT (\n  -- c\n\n  1)"),
        "SELECT (\n  -- c\n\n  1)\n"
    );
}

#[test]
fn start_and_end_of_the_file() {
    assert_eq!(fmt("\n\n  \n  SELECT 1"), "  SELECT 1\n");
    assert_eq!(fmt("\u{feff}\n\nSELECT 1\n\n\n"), "\u{feff}SELECT 1\n");
    assert_eq!(fmt("SELECT 1 -- c"), "SELECT 1 -- c\n");
    assert_eq!(fmt("-- only\n\n"), "-- only\n");
    assert_eq!(fmt(""), "");
    assert_eq!(fmt(" \n\t\r\n"), "");
    assert_eq!(fmt("\u{feff}"), "");
}

/// ClickHouse lexes the first and last bytes of the input specially, so the
/// whitespace there stays where removing or adding it would change a token.
#[test]
fn whitespace_that_decides_a_token_at_the_ends() {
    // `1e` at the very end is a bare word, elsewhere a number.
    assert_eq!(fmt("SELECT 1e"), "SELECT 1e");
    assert_eq!(fmt("SELECT 1e  "), "SELECT 1e\n");
    // `1e+` at the very end is `1e` and `+`, elsewhere one number.
    assert_eq!(fmt("SELECT 1e+"), "SELECT 1e+");
    // `.` at the very start begins a number.
    assert_eq!(fmt("\n\n.x"), "\n\n.x\n");
    assert_eq!(fmt("\n\n.5"), ".5\n");
    // Rendered, `.e` starts the text and is the number `.e` before a
    // newline, `.` and `e` without one.
    assert_eq!(fmt_in(" {%- i -%}.e", true), " {%- i -%}.e");
    assert_eq!(
        fmt_in("\n\n{% x %}{# y #}.e", true),
        "\n\n{% x %}{# y #}.e\n"
    );
    // `-%}` strips the space, so once rendered `.e` follows only the line
    // break at the start, which stays.
    assert_eq!(fmt_in("\n{% x -%} .e\n", true), "\n{% x -%} .e\n");
}

#[test]
fn line_comments() {
    assert_eq!(fmt("SELECT 1 -- c  \t\r\n"), "SELECT 1 -- c\n");
    assert_eq!(fmt("-- c\r\n-- d \r\n"), "-- c\n-- d\n");
    assert_eq!(fmt("SELECT 1 --\r\n"), "SELECT 1 --\n");
    // `#` must keep its space to stay a comment.
    assert_eq!(fmt("SELECT 1 #   \n"), "SELECT 1 #   \n");
    assert_eq!(fmt("SELECT 1 # c  \n"), "SELECT 1 # c\n");
    assert_eq!(fmt("#!  \nSELECT 1"), "#!\nSELECT 1\n");
    // Unicode whitespace ends a comment's line as ASCII whitespace does.
    assert_eq!(fmt("-- c\u{a0}\u{2003}\n"), "-- c\n");
    // In a template, U+FEFF is not whitespace to the renderer and keeps the
    // `-%}` from stripping the line break, so it stays.
    // At the end of the file nothing follows that could be stripped.
    let template = "-- {%- x -%}\u{feff}\nSELECT 1\u{feff}\n";
    assert_eq!(fmt_in(template, true), "-- {%- x -%}\u{feff}\nSELECT 1\n");
    assert_eq!(fmt(template), "-- {%- x -%}\nSELECT 1\n");
}

#[test]
fn multi_line_tokens_keep_their_bytes() {
    assert_eq!(
        fmt("SELECT /* a  \r\n b */ 1\r\n"),
        "SELECT /* a  \r\n b */ 1\n"
    );
    assert_eq!(
        fmt("SELECT 'a  \r\n\r\n\r\n\r\nb'  \r\n"),
        "SELECT 'a  \r\n\r\n\r\n\r\nb'\n"
    );
    assert_eq!(
        fmt("SELECT $$a  \n\n\n\nb$$, `x  \r\ny`, \"p \r\nq\"\r\n"),
        "SELECT $$a  \n\n\n\nb$$, `x  \r\ny`, \"p \r\nq\"\n"
    );
}

#[test]
fn jinja_tags_keep_their_bytes() {
    assert_eq!(
        fmt_in("SELECT {{ x  \r\n }}  \r\n{% if a  \r\n %}\r\n", true),
        "SELECT {{ x  \r\n }}\n{% if a  \r\n %}\n"
    );
    // A line comment with a tag spanning lines: the tag is untouched, the
    // comment's end is trimmed.
    assert_eq!(
        fmt_in("-- {{ x  \r\n }}  \r\nSELECT 1", true),
        "-- {{ x  \r\n }}\nSELECT 1\n"
    );
    // In SQL mode the braces are SQL and the same text is cleaned up.
    assert_eq!(
        fmt("-- {{ x  \r\n }}  \r\nSELECT 1"),
        "-- {{ x\n }}\nSELECT 1\n"
    );
}

#[test]
fn whitespace_next_to_a_strip_marker() {
    // The renderer strips the comment's trailing spaces and the blank lines
    // before `{%-` anyway. What follows the tag meets the comment once
    // rendered, not the tag, so it stays as it is.
    let source = "SELECT 1\nWHERE true -- c  \n\n\n  {%- if a %}  \n\n\n  AND a\n  {%- endif %}\n";
    assert_eq!(
        fmt_in(source, true),
        "SELECT 1\nWHERE true -- c\n\n  {%- if a %}  \n\n\n  AND a\n  {%- endif %}\n"
    );
}

/// Whitespace right after a tag that renders nothing meets, once rendered,
/// whatever comes before the tag; there it can decide a token.
#[test]
fn whitespace_after_a_tag_that_renders_nothing() {
    // `{%- y %}` strips the space after `#`; the space after the tag makes
    // `#` a comment once rendered, inside the template's comment `# {%- y %} `.
    let comment = "{% x %}# {%- y %} \n1\n";
    assert_eq!(fmt_in(comment, true), comment);
    let comment = "{% x %}# {#- y #} \n1\n";
    assert_eq!(fmt_in(comment, true), comment);
    let whitespace = "SELECT 1{% x %}  \n\n\n2\n";
    assert_eq!(fmt_in(whitespace, true), whitespace);
    // Where only whitespace renders before the tag, no render puts anything
    // else before the run.
    assert_eq!(fmt_in("\n{% x %}\n  \n2\n", true), "{% x %}\n\n2\n");
    // After whitespace with only comment tags between, after an expression,
    // or where a `-` marker strips the run, the cleanup applies.
    assert_eq!(
        fmt_in("SELECT 1 {# c #}  \n\n\n2  \n", true),
        "SELECT 1 {# c #}\n\n2\n"
    );
    assert_eq!(
        fmt_in("SELECT {{ x }}  \n\n\n2\n", true),
        "SELECT {{ x }}\n\n2\n"
    );
    assert_eq!(
        fmt_in("SELECT 1 {% x -%}  \n\n\n2\n", true),
        "SELECT 1 {% x -%}\n\n2\n"
    );
}

/// Removing block tags can join template tokens into one literal of the
/// rendered SQL; the whitespace inside it is then the literal's text.
#[test]
fn whitespace_inside_a_rendered_literal() {
    // Two words in the template, one heredoc once rendered.
    let heredoc = "SELECT {% if a %}$x$ \n\n\n {% endif %}$x$  \n";
    assert_eq!(
        fmt_in(heredoc, true),
        "SELECT {% if a %}$x$ \n\n\n {% endif %}$x$\n"
    );
    // `/` and `*` in the template, a block comment once rendered.
    let comment = "SELECT 1 /{# c #}* x  \r\n\n\n *{# c #}/ + 2  \n";
    assert_eq!(
        fmt_in(comment, true),
        "SELECT 1 /{# c #}* x  \r\n\n\n *{# c #}/ + 2\n"
    );
    // The template's data ends at the blank line, which `{%-` strips: the
    // rendered data runs to the end of the file, trailing spaces included.
    let data = "INSERT INTO t FORMAT CSV 1\n\n{%- x %}2  \n";
    assert_eq!(fmt_in(data, true), data);
    // `INSERT{# c #}` is one word of the template and `INSERT` once rendered;
    // the blank lines after the format name decide where its data starts.
    let insert = "INSERT{# c #} INTO t FORMAT CSV\n\n\nx  \n";
    assert_eq!(
        fmt_in(insert, true),
        "INSERT{# c #} INTO t FORMAT CSV\n\n\nx\n"
    );
}

/// A branch that is left out can move where an `INSERT`'s data ends, so in
/// a template with block tags, the text from such an `INSERT` on stays.
#[test]
fn template_data_that_a_branch_can_move_keeps_every_byte() {
    // With `x` false, `b` is data that ends at the blank line; the
    // rendered text with every branch taken shows no data.
    let source = "SELECT 1;  \n{% if x %}'a' {% endif %}INSERT INTO t FORMAT TSV\nb\n\n\n";
    assert_eq!(
        fmt_in(source, true),
        "SELECT 1;\n{% if x %}'a' {% endif %}INSERT INTO t FORMAT TSV\nb\n\n\n"
    );
}

#[test]
fn values_data_keeps_every_byte() {
    let source = "INSERT INTO t VALUES (1, 'a'),  \r\n\r\n\r\n  (2, 'b')  ;  \r\nSELECT 1;  \r\n";
    assert_eq!(
        fmt(source),
        "INSERT INTO t VALUES (1, 'a'),  \r\n\r\n\r\n  (2, 'b')  ;\nSELECT 1;\n"
    );
    assert_eq!(
        fmt("INSERT INTO t VALUES\r\n\r\n\r\n(1)  \r\n"),
        "INSERT INTO t VALUES\n\n(1)\n"
    );
}

#[test]
fn format_data_keeps_every_byte() {
    let source = "INSERT INTO t FORMAT CSV  \r\n1, 2  \r\n\r\n3\n\n\n\n\nSELECT 1;\n";
    assert_eq!(
        fmt(source),
        "INSERT INTO t FORMAT CSV\n1, 2  \r\n\r\n3\n\n\nSELECT 1;\n"
    );
    // Data that runs to the end of the file gets no final newline.
    assert_eq!(
        fmt("INSERT INTO t FORMAT TSV\n1\t\\N  "),
        "INSERT INTO t FORMAT TSV\n1\t\\N  "
    );
    // Data that ends at a blank line keeps it at the end of the file.
    assert_eq!(
        fmt("INSERT INTO t FORMAT CSV\n1\n\n\n\n"),
        "INSERT INTO t FORMAT CSV\n1\n\n"
    );
    // A blank line right after the name leaves no data.
    assert_eq!(
        fmt("INSERT INTO t FORMAT CSV\n\n\n\n\n1,2\n"),
        "INSERT INTO t FORMAT CSV\n\n\n1,2\n"
    );
}

#[test]
fn lexer_errors_leave_the_text_alone() {
    let options = Options::default();
    let Err(Error::Lex(e)) = format("SELECT 1;\nSELECT 'a", &options) else {
        panic!("formatted");
    };
    assert_eq!(e.kind, TokenKind::ErrorSingleQuoteIsNotClosed);
    assert_eq!((e.position.line, e.position.column), (2, 8));
    assert_eq!(e.to_string(), "2:8: Single quoted string is not closed");
    // An error in data ClickHouse does not lex is no error.
    assert!(format("INSERT INTO t FORMAT CSV\n'a\n", &options).is_ok());
    let jinja = options.with_jinja(true);
    assert!(matches!(
        format("SELECT 1 -- {# x\nSELECT 2", &jinja),
        Err(Error::Lex(_))
    ));
}

/// Text the parser finds ClickHouse reads as written keeps its whitespace,
/// even at the end of a line.
#[test]
fn literals_read_as_text_are_kept() {
    let source = "SELECT [1,  \n2]::Array(Int8), x  \n";
    let split = crate::split::split(source, crate::lexer::Mode::Sql);
    let parse = crate::syntax::parse_split(source, &split, crate::syntax::Entry::SelectListProbe);
    let cleaned = super::cleanup(source, &split, crate::lexer::Mode::Sql, parse.facts());
    assert_eq!(cleaned, "SELECT [1,  \n2]::Array(Int8), x\n");
}
