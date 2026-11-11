//! `chairlift::format` on the vendored fixtures.

use std::fs;
use std::path::Path;

use chairlift::Options;
use chairlift::lexer::Mode;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn options_for(name: &str) -> Options {
    Options::default().with_jinja(Mode::for_path(Path::new(name)) == Mode::Jinja)
}

/// The DESIGN.md example's expected layout is formatted already.
#[test]
fn the_design_example_as_it_should_print_is_formatted() {
    let name = "design/page_views_1h.expected.sql.j2";
    let source = fixture(name);
    assert_eq!(
        chairlift::format(&source, &options_for(name)).unwrap(),
        source
    );
    assert!(chairlift::check(&source, &options_for(name)).unwrap());
}

/// Every vendored fixture, formatted: a snapshot each, and formatting the
/// output again changes nothing.
#[test]
fn fixtures_format_as_their_snapshots_show() {
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|e| e.expect("a directory entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("sql" | "j2")
            ) {
                out.push(path);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files = Vec::new();
    walk(&root, &mut files);
    assert!(files.len() >= 14, "{} fixtures", files.len());
    for path in files {
        let name = path
            .strip_prefix(&root)
            .expect("under the fixtures")
            .to_string_lossy()
            .replace('\\', "/");
        let options = options_for(&name);
        let source = fixture(&name);
        let once = chairlift::format(&source, &options).unwrap_or_else(|e| panic!("{name}: {e}"));
        let twice = chairlift::format(&once, &options).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(twice, once, "{name} is not idempotent");
        insta::assert_snapshot!(name.replace('/', "__"), once);
    }
}

#[test]
fn check_reports_whether_a_text_is_formatted() {
    let options = Options::default();
    assert!(chairlift::check("SELECT 1;\n", &options).unwrap());
    assert!(!chairlift::check("SELECT 1; \n", &options).unwrap());
    assert!(!chairlift::check("SELECT 1;", &options).unwrap());
    assert!(chairlift::check("SELECT 'x", &options).is_err());
}

#[test]
fn line_width_decides_where_lines_break() {
    let source = "SELECT x FROM t GROUP BY a, b\n";
    assert_eq!(
        chairlift::format(source, &Options::default()).unwrap(),
        "SELECT x\nFROM t\nGROUP BY a, b\n"
    );
    let narrow = Options::default().with_line_width(10);
    assert_eq!(
        chairlift::format(source, &narrow).unwrap(),
        "SELECT x\nFROM t\nGROUP BY\n    a,\n    b\n"
    );
}

/// What the layout did with each statement.
#[test]
fn each_statement_is_formatted_or_kept_with_a_reason() {
    use chairlift::layout::Decision;
    use chairlift::layout::Kept;
    let (output, statements) =
        chairlift::format_with_statements("select 1; system flush logs; ;", &Options::default())
            .unwrap();
    assert_eq!(output, "SELECT 1;\nsystem flush logs; ;\n");
    let decisions: Vec<Decision> = statements.iter().map(|s| s.decision).collect();
    assert_eq!(
        decisions,
        [
            Decision::Formatted,
            Decision::Kept(Kept::NotParsed),
            Decision::Formatted
        ]
    );
    assert_eq!(statements[1].tokens, 5..11);
}

/// Outside Jinja mode, tag delimiters are SQL text: in a string, and as two
/// braces of a map in a map.
#[test]
fn plain_sql_with_braces_is_not_a_template() {
    use chairlift::layout::Decision;
    let source = "select '{% if x %}' as a, '{{ y }}' as b, {'k': {'n': 1}} from t";
    let (output, statements) =
        chairlift::format_with_statements(source, &Options::default()).unwrap();
    assert_eq!(
        output,
        "SELECT\n    '{% if x %}' AS a,\n    '{{ y }}' AS b,\n    {'k': {'n': 1}}\nFROM t\n"
    );
    assert_eq!(statements[0].decision, Decision::Formatted);
}

/// Every tag of a template comes out byte for byte and in order, with the
/// output verified: a tag inside a string, tags glued to identifiers, Jinja
/// comments, `+` and `-` markers and a raw block.
#[test]
fn every_tag_of_a_template_keeps_its_bytes() {
    use chairlift::lexer::jinja::tags;
    let source = "{# a header #}\n{{ config( x = 1 ) }}\n\
        select events_{{ suffix }}, {{x}}_n , '{% if a %}A{% else %}B{% endif %}' as s,\n\
        {%+ if b +%} c, {%+ endif +%} {%- raw -%} '{{ not a tag }}' {%- endraw -%} as r\n\
        from {{ db | sql_ident }}.t {#- trailing #} where d = 1;\n";
    let options = Options::default().with_jinja(true);
    let output = chairlift::format(source, &options).unwrap();
    let texts =
        |text: &str| -> Vec<String> { tags(text).map(|t| text[t.range].to_owned()).collect() };
    assert_eq!(texts(&output), texts(source), "{output}");
    assert!(output.contains("SELECT"), "{output}");
    let parse = chairlift::syntax::parse(source, Mode::Jinja);
    chairlift::verify::verify(source, &output, Mode::Jinja, parse.facts())
        .unwrap_or_else(|e| panic!("{e}\n{output}"));
}
