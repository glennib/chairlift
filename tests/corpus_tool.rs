//! The `corpus` tool, end to end: over the vendored fixtures with the oracle
//! when `clickhouse-format` is available, and over a small corpus of its own
//! with one file of each outcome.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

fn corpus(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_corpus"))
        .args(args)
        .output()
        .expect("the corpus tool runs")
}

/// A fresh directory for a test; removed when dropped.
fn temp_dir(test: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!("chairlift-corpus-{}-{test}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    TempDir(dir)
}

struct TempDir(PathBuf);

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn oracle_available() -> bool {
    Command::new("clickhouse-format")
        .arg("--help")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The CSV's rows, as maps from column to value; no field holds a comma.
fn rows(csv: &str) -> Vec<Vec<(String, String)>> {
    let mut lines = csv.lines();
    let header: Vec<&str> = lines.next().unwrap().split(',').collect();
    lines
        .map(|line| {
            header
                .iter()
                .zip(line.split(','))
                .map(|(h, v)| ((*h).to_owned(), v.to_owned()))
                .collect()
        })
        .collect()
}

fn field<'a>(row: &'a [(String, String)], name: &str) -> &'a str {
    &row.iter().find(|(h, _)| h == name).unwrap().1
}

#[test]
fn fixtures_verify_and_keep_their_meaning() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let dir = temp_dir("fixtures");
    let csv = dir.join("out.csv");
    let corpus_arg = format!("fixtures={}", fixtures.display());
    let mut args = vec!["--corpus", &corpus_arg, "--csv", csv.to_str().unwrap()];
    let oracle = oracle_available();
    if oracle {
        args.push("--oracle");
    } else {
        eprintln!("skipping the oracle: clickhouse-format is unavailable");
    }
    let output = corpus(&args);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.starts_with("corpus "), "{stdout}");
    let csv = fs::read_to_string(&csv).unwrap();
    assert!(csv.starts_with(
        "corpus,path,mode,bytes,tokens,statements,outcome,changed,idempotent,format_us,oracle,\
         oracle_class,oracle_statements,oracle_judged,oracle_same,oracle_differs,\
         oracle_output_rejected,oracle_rejected_unrendered_jinja,oracle_rejected_insert_data,\
         oracle_rejected_empty_query,oracle_rejected_syntax,oracle_rejected_other,\
         oracle_rejected_failed,oracle_changed,oracle_changed_judged,round_trip,\
         parsed_statements,opaque_statements,partial_statements,verbatim_statements,clauses,\
         verbatim_clauses,items,verbatim_items,formatted_statements,kept_statements,detail\n"
    ));
    let rows = rows(&csv);
    assert_eq!(rows.len(), 17);
    for row in &rows {
        assert_eq!(field(row, "outcome"), "formatted");
        assert_eq!(field(row, "idempotent"), "true");
        assert_eq!(field(row, "round_trip"), "true");
        if oracle {
            assert!(
                matches!(field(row, "oracle"), "same" | "unjudgeable"),
                "{row:?}"
            );
            let count = |name| field(row, name).parse::<usize>().unwrap();
            assert_eq!(count("oracle_statements"), count("statements"), "{row:?}");
            assert_eq!(count("oracle_differs"), 0, "{row:?}");
        } else {
            assert_eq!(field(row, "oracle_statements"), "");
        }
    }
    let design = rows
        .iter()
        .find(|r| field(r, "path") == "design/page_views_1h.sql.j2")
        .unwrap();
    assert_eq!(field(design, "mode"), "jinja");
    // Formatting the example re-cases its keywords; its expected output is
    // formatted already.
    assert_eq!(field(design, "changed"), "true");
    let expected = rows
        .iter()
        .find(|r| field(r, "path") == "design/page_views_1h.expected.sql.j2")
        .unwrap();
    assert_eq!(field(expected, "changed"), "false");
    if oracle {
        // The template is unjudgeable as a whole, but its statement without
        // tags is judged.
        assert_eq!(field(design, "oracle_class"), "unrendered_jinja");
        assert_eq!(field(design, "oracle_judged"), "1");
        assert_eq!(field(design, "oracle_rejected_unrendered_jinja"), "2");
        assert!(stdout.contains("judged %"), "{stdout}");
    }
}

#[test]
fn every_outcome_is_recorded() {
    let dir = temp_dir("outcomes");
    let root = dir.join("c");
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(root.join("messy.sql"), "SELECT 1;  \n\n\n\n").unwrap();
    fs::write(root.join("sub/broken.sql"), "SELECT 'x").unwrap();
    fs::write(root.join("latin1.sql"), b"SELECT '\xe9'").unwrap();
    fs::write(root.join("t.sql.j2"), "SELECT {{ x }}\n").unwrap();
    fs::write(root.join("notes.txt"), "not SQL").unwrap();
    let csv = dir.join("out.csv");
    let corpus_arg = format!("mine={}", root.display());
    let output = corpus(&[
        "--corpus",
        &corpus_arg,
        "--csv",
        csv.to_str().unwrap(),
        "--jobs",
        "1",
    ]);
    assert_eq!(output.status.code(), Some(0));
    let csv = fs::read_to_string(&csv).unwrap();
    let outcomes: Vec<(String, String, String)> = rows(&csv)
        .iter()
        .map(|r| {
            (
                field(r, "path").to_owned(),
                field(r, "mode").to_owned(),
                field(r, "outcome").to_owned(),
            )
        })
        .collect();
    let expected = [
        ("latin1.sql", "sql", "not_utf8"),
        ("messy.sql", "sql", "formatted"),
        ("sub/broken.sql", "sql", "lex_error"),
        ("t.sql.j2", "jinja", "formatted"),
    ];
    assert_eq!(
        outcomes,
        expected.map(|(p, m, o)| (p.to_owned(), m.to_owned(), o.to_owned()))
    );
    assert!(csv.contains(",formatted,true,true,"), "{csv}");
}

/// The re-casing modes: the expression probe parses `SELECT <list>`, its
/// keyword slots verify, and the statements CSV says how each statement
/// went.
#[test]
fn the_expression_probe_reports_each_statement() {
    let dir = temp_dir("probe");
    let root = dir.join("c");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("q.sql"),
        "select 1 as x, left(s, 2);\nselect f(a b);\ndrop table t;\nselect sum(x) over w;\n\
         select 1, sum(x) over (order by y);\n",
    )
    .unwrap();
    let csv = dir.join("statements.csv");
    let corpus_arg = format!("mine={}", root.display());
    let output = corpus(&[
        "--corpus",
        &corpus_arg,
        "--recase",
        "expressions",
        "--statements-csv",
        csv.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("probed statements"), "{stdout}");
    let csv = fs::read_to_string(&csv).unwrap();
    let mut lines = csv.lines();
    assert_eq!(
        lines.next(),
        Some(
            "corpus,path,statement,kind,outcome,layout,clauses,verbatim_clauses,items,\
             verbatim_items,opaque,probe_items,probe_opaque_items,probe_fallback_items,fallback,\
             text"
        )
    );
    let rows: Vec<&str> = lines.collect();
    // Re-casing lays out nothing, so the layout column is empty.
    assert!(rows[0].starts_with("mine,q.sql,1,select,parsed,,"), "{csv}");
    assert!(
        rows[1].starts_with("mine,q.sql,2,select,partial,,"),
        "{csv}"
    );
    assert!(rows[1].contains("expected"), "{csv}");
    assert!(
        rows[2].starts_with("mine,q.sql,3,drop table,verbatim,,"),
        "{csv}"
    );
    assert!(rows[3].starts_with("mine,q.sql,4,select,parsed,,"), "{csv}");
    assert!(
        rows[4].starts_with("mine,q.sql,5,select,opaque,,0,0,3,0,1,2,1,0,"),
        "{csv}"
    );

    let output = corpus(&["--corpus", &corpus_arg, "--recase", "all"]);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn trees_are_printed() {
    let dir = temp_dir("tree");
    let file = dir.join("t.sql");
    fs::write(&file, "SELECT 1;\n").unwrap();
    let output = corpus(&["--tree", file.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with("Root@0..10\n  Statement@0..9\n"),
        "{stdout}"
    );
}

#[test]
fn a_missing_corpus_is_an_error() {
    let output = corpus(&["--corpus", "/nonexistent/chairlift-corpus"]);
    assert_eq!(output.status.code(), Some(2));
    let output = corpus(&[]);
    assert_eq!(output.status.code(), Some(2));
}
