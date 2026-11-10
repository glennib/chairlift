//! End-to-end tests of the `chairlift` binary.

use std::process::Command;
use std::process::Output;

fn chairlift(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_chairlift"))
        .args(args)
        .output()
        .expect("chairlift binary runs")
}

#[test]
fn version_names_the_targeted_clickhouse_release() {
    let output = chairlift(&["--version"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout,
        format!(
            "chairlift {} (ClickHouse {})\n",
            env!("CARGO_PKG_VERSION"),
            chairlift::CLICKHOUSE_VERSION
        )
    );
}

#[test]
fn help() {
    let output = chairlift(&["--help"]);
    assert!(output.status.success());
    insta::assert_snapshot!(String::from_utf8(output.stdout).unwrap(), @r"
    Format ClickHouse SQL and Jinja-templated SQL files

    Usage: chairlift [PATHS]...

    Arguments:
      [PATHS]...  Files or directories to format; `-` reads stdin and writes stdout

    Options:
      -h, --help     Print help
      -V, --version  Print version
    ");
}

#[test]
fn paths_are_rejected_until_formatting_exists() {
    let output = chairlift(&["query.sql"]);
    assert_eq!(output.status.code(), Some(2));
}
