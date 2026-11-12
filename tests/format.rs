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

/// Text kept as written keeps the indentation of its later lines whether or
/// not the file starts with a byte-order mark and whether or not it starts
/// on the first line.
#[test]
fn a_byte_order_mark_does_not_shift_text_kept_as_written() {
    let options = Options::default();
    for source in ["\u{feff}FROM t[-- c\n *", "\u{feff}FROM\nt[-- c\n *"] {
        let once = chairlift::format(source, &options).unwrap();
        assert_eq!(once, "\u{feff}FROM t[-- c\n *\n", "{source:?}");
        assert_eq!(chairlift::format(&once, &options).unwrap(), once);
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

/// Formats `source` on a thread of its own with the stack of a process's main thread,
/// so that a recursion the depth limit does not bound overflows here as it
/// would in the binary.
fn spawn_format_on_main_sized_stack(
    source: String,
) -> std::thread::JoinHandle<Result<String, String>> {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || chairlift::format(&source, &Options::default()).map_err(|e| e.to_string()))
        .expect("thread starts")
}

/// Pairs nested in a dictionary's `SOURCE` deeper than the parser's depth
/// limit are kept as written; this depth overflowed the stack.
#[test]
fn deeply_nested_dictionary_pairs_are_kept_without_overflowing() {
    let deep = 100_000;
    let source = format!(
        "CREATE DICTIONARY d (a UInt64) PRIMARY KEY a SOURCE({}a 1{}) LAYOUT(FLAT()) LIFETIME(0);\n",
        "x(".repeat(deep),
        ")".repeat(deep)
    );
    let output = spawn_format_on_main_sized_stack(source)
        .join()
        .expect("no overflow")
        .unwrap();
    assert!(output.contains(&"x(".repeat(deep)));
}

/// Every bracketed construct of the statements beyond `SELECT` and
/// `CREATE TABLE`, nested far deeper than the depth limit, formats without
/// overflowing the stack.
#[test]
fn deep_nesting_in_any_statement_formats_without_overflowing() {
    let deep = 200_000;
    // The text before the nesting, the opener repeated, what is innermost,
    // the closer repeated, and the text after it.
    let cases = [
        "INSERT INTO t |(|a|)| VALUES (1)",
        "INSERT INTO t (a) VALUES |(|1|)|",
        "INSERT INTO t |(|SELECT 1|)|",
        "INSERT INTO t SELECT * FROM |(|SELECT 1|)|",
        "INSERT INTO FUNCTION |f(|1|)| SELECT 1",
        "ALTER TABLE t |(|a|)|",
        "ALTER TABLE t UPDATE a = |(|1|)| WHERE 1",
        "ALTER TABLE t DELETE WHERE |(|1|)|",
        "ALTER TABLE t ADD COLUMN a |Array(|Int8|)|",
        "ALTER TABLE t MODIFY COLUMN a Int8 DEFAULT |[|1|]|",
        "ALTER TABLE t MODIFY COLUMN a Int8 CODEC|(|ZSTD|)|",
        "ALTER TABLE t ADD PROJECTION p |(|SELECT 1|)|",
        "ALTER TABLE t ADD INDEX i |(|a|)| TYPE minmax",
        "ALTER TABLE t MODIFY TTL |(|d|)|",
        "ALTER TABLE t MODIFY SETTING a = |(|1|)|",
        "ALTER TABLE t DROP PARTITION |(|1|)|",
        "ALTER TABLE t MODIFY QUERY SELECT |(|1|)|",
        "ALTER TABLE t MODIFY ORDER BY |(|a|)|",
        "DELETE FROM t WHERE |(|1|)|",
        "UPDATE t SET a = |(|1|)| WHERE 1",
        "CREATE VIEW v AS |(|SELECT 1|)|",
        "CREATE DATABASE d ENGINE = e|(|1|)|",
        "CREATE DATABASE d ENGINE = e(|f(|1|)|)",
        "CREATE DICTIONARY d (a |Array(|Int8|)|) PRIMARY KEY a",
        "CREATE DICTIONARY d (a Int8 DEFAULT |[|1|]|) PRIMARY KEY a",
        "CREATE DICTIONARY d (a Int8 EXPRESSION |(|1|)|) PRIMARY KEY a",
        "CREATE DICTIONARY d (a Int8) SOURCE(|x(|a 1|)|)",
        "CREATE DICTIONARY d (a Int8) SOURCE(x(|k(|a 1|)|))",
        "CREATE DICTIONARY d (a Int8) SOURCE(x(k |f(|1|)|))",
        "CREATE DICTIONARY d (a Int8) SOURCE(x(k |[|1|]|))",
        "CREATE DICTIONARY d (a Int8) LAYOUT(|x(|a 1|)|)",
        "CREATE DICTIONARY d (a Int8) LIFETIME(MIN |x(|a 1|)|)",
        "CREATE DICTIONARY d (a Int8) RANGE(MIN |x(|a 1|)|)",
        "CREATE DICTIONARY d (a Int8) SETTINGS(a = |(|1|)|)",
        "CREATE FUNCTION f AS x -> |(|1|)|",
        "CREATE FUNCTION f AS |x -> |1||",
        "CREATE INDEX i ON t |(|a|)| TYPE minmax",
        "CREATE INDEX i ON t (a) TYPE x|(|1|)|",
        "GRANT SELECT|(|a|)| ON t TO u",
        "GRANT SELECT ON t TO u|(|a|)|",
        "REVOKE SELECT|(|a|)| ON t FROM u",
        "CREATE ROW POLICY p ON t USING |(|1|)| TO u",
        "CREATE USER u SETTINGS a = |(|1|)|",
        "CREATE USER u HOST IP |(|'x'|)|",
        "CREATE ROLE r SETTINGS a = |[|1|]|",
        "CREATE SETTINGS PROFILE p SETTINGS a MIN |(|1|)|",
        "CREATE QUOTA q FOR INTERVAL |(|1|)| HOUR MAX queries = 1",
        "CREATE QUOTA q FOR INTERVAL 1 HOUR MAX queries = |(|1|)|",
        "ALTER USER u SETTINGS a = |(|1|)|",
        "SET a = |[|1|]|",
        "SET a = |(|1|)|",
        "SET ROLE |(|r|)|",
        "DROP TABLE t SETTINGS a = |(|1|)|",
        "OPTIMIZE TABLE t PARTITION |(|1|)|",
        "OPTIMIZE TABLE t DEDUPLICATE BY |(|a|)|",
        "ATTACH TABLE t FROM '/x' (a |Array(|Int8|)|) ENGINE = Memory",
        "RENAME TABLE |(|a|)| TO b",
        "DESCRIBE |(|SELECT 1|)|",
        "DESCRIBE TABLE |f(|1|)|",
        "DESCRIBE TABLE t SETTINGS a = |(|1|)|",
        "SHOW TABLES LIKE |(|'x'|)|",
        "SHOW TABLES WHERE |(|1|)|",
        "SHOW TABLES LIMIT |(|1|)|",
        "SHOW CREATE TABLE |(|t|)|",
        "SHOW GRANTS FOR |(|u|)|",
    ];
    let runs: Vec<_> = cases
        .iter()
        .map(|case| {
            let [head, open, inner, close, tail] = case.split('|').collect::<Vec<_>>()[..] else {
                panic!("{case}: five parts");
            };
            let source = format!(
                "{head}{}{inner}{}{tail};\n",
                open.repeat(deep),
                close.repeat(deep)
            );
            (case, spawn_format_on_main_sized_stack(source))
        })
        .collect();
    for (case, run) in runs {
        let output = run.join().unwrap_or_else(|_| panic!("{case}: no overflow"));
        assert!(output.is_ok(), "{case}: {output:?}");
    }
}
