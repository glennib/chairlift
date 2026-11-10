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

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chairlift-corpus-{}-{test}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
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
    let csv = temp_dir("fixtures").join("out.csv");
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
         oracle_class,detail\n"
    ));
    let rows = rows(&csv);
    assert_eq!(rows.len(), 14);
    for row in &rows {
        assert_eq!(field(row, "outcome"), "formatted");
        assert_eq!(field(row, "idempotent"), "true");
        if oracle {
            assert!(
                matches!(field(row, "oracle"), "same" | "unjudgeable"),
                "{row:?}"
            );
        }
    }
    let design = rows
        .iter()
        .find(|r| field(r, "path") == "design/page_views_1h.sql.j2")
        .unwrap();
    assert_eq!(field(design, "mode"), "jinja");
    assert_eq!(field(design, "changed"), "false");
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

#[test]
fn a_missing_corpus_is_an_error() {
    let output = corpus(&["--corpus", "/nonexistent/chairlift-corpus"]);
    assert_eq!(output.status.code(), Some(2));
    let output = corpus(&[]);
    assert_eq!(output.status.code(), Some(2));
}
