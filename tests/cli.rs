//! End-to-end tests of the `chairlift` binary.

use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

fn chairlift(args: &[&str]) -> Output {
    run(args, None, None)
}

fn run(args: &[&str], stdin: Option<&[u8]>, dir: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_chairlift"));
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let mut child = command.spawn().expect("chairlift binary runs");
    let mut pipe = child.stdin.take().unwrap();
    if let Some(input) = stdin {
        pipe.write_all(input).unwrap();
    }
    drop(pipe);
    child.wait_with_output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

/// A fresh directory for a test, outside any repository so that no
/// `.gitignore` above it applies.
fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chairlift-cli-{}-{test}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, contents).unwrap();
    path
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

const MESSY: &str = "SELECT 1;  \r\n\n\n\n\nSELECT 2;";
const CLEAN: &str = "SELECT 1;\n\n\nSELECT 2;\n";

#[test]
fn version_names_the_targeted_clickhouse_release() {
    let output = chairlift(&["--version"]);
    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
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
    // clap indents blank lines inside an option's help; trailing whitespace
    // does not survive in a snapshot.
    let help = stdout(&output);
    let help: Vec<&str> = help.lines().map(str::trim_end).collect();
    insta::assert_snapshot!(help.join("\n"), @r"
    Format ClickHouse SQL and Jinja-templated SQL files.

    Files are formatted in place. Directories are searched for `*.sql`, `*.sql.j2` and `*.j2.sql` files, skipping hidden files and whatever `.gitignore` and `.ignore` files exclude.

    Usage: chairlift [OPTIONS] <PATHS>...

    Arguments:
      <PATHS>...
              Files or directories to format; `-` reads stdin and writes stdout

    Options:
          --check
              Write nothing; list the files that would change and exit with 1 if there are any

          --diff
              Write nothing; print a unified diff of the changes and exit with 1 if there are any

          --line-width <WIDTH>
              The width to lay lines out to

              [default: 100]

          --jinja <WHEN>
              Whether files are Jinja templates; `auto` decides by name (`*.sql.j2` and `*.j2.sql` are), and stdin is not

              [default: auto]
              [possible values: auto, on, off]

      -h, --help
              Print help (see a summary with '-h')

      -V, --version
              Print version

    Exit status: 0 when done, 1 when --check or --diff found files that would change, 2 when a file could not be formatted (unreadable, not UTF-8, a lexer error, or a failed verification) or the arguments are wrong.
    ");
}

#[test]
fn paths_are_required() {
    let output = chairlift(&[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("<PATHS>"));
}

#[test]
fn bad_arguments_exit_with_2() {
    assert_eq!(
        chairlift(&["--line-width", "0", "x.sql"]).status.code(),
        Some(2)
    );
    assert_eq!(
        chairlift(&["--jinja", "maybe", "x.sql"]).status.code(),
        Some(2)
    );
    let output = chairlift(&["-", "x.sql"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("`-` (stdin) cannot be combined"));
}

#[test]
fn stdin_to_stdout() {
    let output = run(&["-"], Some(MESSY.as_bytes()), None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), CLEAN);
    let output = run(&["--line-width", "80", "-"], Some(CLEAN.as_bytes()), None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), CLEAN);
}

#[test]
fn stdin_check_and_diff() {
    let output = run(&["--check", "-"], Some(MESSY.as_bytes()), None);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "Would reformat: -\n");
    let output = run(&["--check", "-"], Some(CLEAN.as_bytes()), None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "");

    let output = run(&["--diff", "-"], Some(b"SELECT 1;  \n"), None);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "--- a/-\n+++ b/-\n@@ -1 +1 @@\n-SELECT 1;  \n+SELECT 1;\n"
    );
}

#[test]
fn stdin_errors() {
    let output = run(&["-"], Some(b"SELECT 1;\nSELECT 'x"), None);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: <stdin>:2:8: Single quoted string is not closed; the file is left unchanged\n"
    );
    let output = run(&["-"], Some(b"SELECT '\xff'"), None);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("not UTF-8 (at byte 8)"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn stdin_jinja_mode() {
    // A string holding a tag with a quote in it: one string in Jinja mode,
    // an unclosed string in SQL mode.
    let template = "SELECT '{{ \"'\" }}';\n";
    assert_eq!(
        run(&["-"], Some(template.as_bytes()), None).status.code(),
        Some(2)
    );
    let output = run(&["--jinja", "on", "-"], Some(template.as_bytes()), None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), template);
}

#[test]
fn files_are_formatted_in_place() {
    let dir = temp_dir("in-place");
    let messy = write(&dir, "messy.sql", MESSY);
    let clean = write(&dir, "clean.sql", CLEAN);
    let output = chairlift(&[messy.to_str().unwrap(), clean.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(read(&messy), CLEAN);
    assert_eq!(read(&clean), CLEAN);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "1 file reformatted, 1 file already formatted\n"
    );
    // No temporary file is left behind.
    let names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

#[cfg(unix)]
#[test]
fn writing_keeps_permissions_and_symbolic_links() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = temp_dir("permissions");
    let target = write(&dir, "real.sql", MESSY);
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    let link = dir.join("link.sql");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let output = chairlift(&[link.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(read(&target), CLEAN);
    let mode = fs::metadata(&target).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o640);
}

#[test]
fn check_lists_files_and_writes_nothing() {
    let dir = temp_dir("check");
    let messy = write(&dir, "a/messy.sql", MESSY);
    write(&dir, "a/clean.sql", CLEAN);
    let output = run(&["--check", "a"], None, Some(&dir));
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        format!(
            "Would reformat: {}\n",
            Path::new("a").join("messy.sql").display()
        )
    );
    assert_eq!(
        stderr(&output),
        "1 file would be reformatted, 1 file already formatted\n"
    );
    assert_eq!(read(&messy), MESSY);

    fs::write(&messy, CLEAN).unwrap();
    let output = run(&["--check", "a"], None, Some(&dir));
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "");
}

#[test]
fn diff_prints_changes_and_writes_nothing() {
    let dir = temp_dir("diff");
    let messy = write(&dir, "q.sql", "SELECT 1;\n\n\n\nSELECT 2; \n");
    let output = run(&["--diff", "q.sql"], None, Some(&dir));
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "--- a/q.sql\n+++ b/q.sql\n@@ -1,5 +1,4 @@\n SELECT 1;\n \n \n-\n-SELECT 2; \n+SELECT 2;\n"
    );
    assert_eq!(read(&messy), "SELECT 1;\n\n\n\nSELECT 2; \n");
    // `--check` with `--diff` diffs.
    let output = run(&["--check", "--diff", "q.sql"], None, Some(&dir));
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).starts_with("--- a/q.sql\n"));
}

#[test]
fn directories_are_searched_for_sql_files() {
    let dir = temp_dir("discovery");
    for name in [
        "a.sql",
        "b.sql.j2",
        "c.j2.sql",
        "UPPER.SQL",
        "sub/d.sql",
        "notes.txt",
        "ignored.sql",
        "skipped/e.sql",
        ".hidden/f.sql",
        ".g.sql",
    ] {
        write(&dir, name, "SELECT 1; ");
    }
    write(&dir, ".gitignore", "ignored.sql\nskipped/\n");
    let output = run(&["--check", "."], None, Some(&dir));
    assert_eq!(output.status.code(), Some(1));
    let listed: Vec<String> = stdout(&output)
        .lines()
        .map(|l| {
            l.strip_prefix("Would reformat: ")
                .unwrap()
                .replace('\\', "/")
        })
        .collect();
    assert_eq!(
        listed,
        [
            "./UPPER.SQL",
            "./a.sql",
            "./b.sql.j2",
            "./c.j2.sql",
            "./sub/d.sql"
        ]
    );
    // A file named on the command line is formatted whatever its name, and
    // a file named twice is formatted once.
    let output = run(
        &["--check", "notes.txt", "ignored.sql", "notes.txt"],
        None,
        Some(&dir),
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "Would reformat: ignored.sql\nWould reformat: notes.txt\n"
    );
}

#[test]
fn jinja_mode_by_name() {
    let dir = temp_dir("jinja");
    let template = "SELECT '{{ \"'\" }}';\n";
    write(&dir, "t.sql.j2", template);
    write(&dir, "u.j2.sql", template);
    write(&dir, "v.sql", template);
    let output = run(&["--check", "t.sql.j2", "u.j2.sql"], None, Some(&dir));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let output = run(&["--check", "v.sql"], None, Some(&dir));
    assert_eq!(output.status.code(), Some(2));
    let output = run(&["--check", "--jinja", "on", "v.sql"], None, Some(&dir));
    assert_eq!(output.status.code(), Some(0));
    let output = run(&["--check", "--jinja", "off", "t.sql.j2"], None, Some(&dir));
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn errors_leave_files_alone_and_exit_with_2() {
    let dir = temp_dir("errors");
    let broken = write(&dir, "broken.sql", "SELECT 1;  \nSELECT 'x");
    let latin1 = dir.join("latin1.sql");
    fs::write(&latin1, b"SELECT 'caf\xe9';  \n").unwrap();
    let messy = write(&dir, "messy.sql", MESSY);
    let output = run(&["."], None, Some(&dir));
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(read(&broken), "SELECT 1;  \nSELECT 'x");
    assert_eq!(fs::read(&latin1).unwrap(), b"SELECT 'caf\xe9';  \n");
    // The other files are still formatted.
    assert_eq!(read(&messy), CLEAN);
    let errors = stderr(&output).replace('\\', "/");
    assert_eq!(
        errors,
        "error: ./broken.sql:2:8: Single quoted string is not closed; the file is left unchanged\n\
         error: ./latin1.sql: not UTF-8 (at byte 11); the file is left unchanged\n\
         1 file reformatted, 0 files already formatted, 2 errors\n"
    );
    // An error wins over differences found by `--check`.
    fs::write(&messy, MESSY).unwrap();
    assert_eq!(
        run(&["--check", "."], None, Some(&dir)).status.code(),
        Some(2)
    );
}

#[test]
fn a_missing_path_is_an_error() {
    let dir = temp_dir("missing");
    let output = run(&["nope.sql"], None, Some(&dir));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).starts_with("error: nope.sql: cannot be read: "),
        "{}",
        stderr(&output)
    );
}
