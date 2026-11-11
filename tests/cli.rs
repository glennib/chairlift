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
/// `.gitignore` above it applies; removed when dropped.
fn temp_dir(test: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!("chairlift-cli-{}-{test}", std::process::id()));
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
    insta::assert_snapshot!(help.join("\n"), @"
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

          --verify
              Also check with ClickHouse's own formatter, `clickhouse-format`, that ClickHouse reads each changed file as before. A file it reads differently is an error and is not written, and so is a file of which it judges nothing because it crashes, times out or prints too much on it; where that happens to only some statements of a file, a warning that --quiet does not hide says so. Files that formatting leaves unchanged are not judged, as there is nothing to compare; neither are statements it cannot parse, nor template statements that hold Jinja

          --clickhouse-binary <PATH>
              The `clickhouse-format` program for --verify, or the `clickhouse` binary [default: `clickhouse-format` on PATH]

          --line-width <WIDTH>
              The width to lay lines out to

              [default: 100]

          --jinja <WHEN>
              Whether files are Jinja templates; `auto` decides by name (`*.sql.j2` and `*.j2.sql` are), and stdin is not unless --stdin-filename names a template

              [default: auto]
              [possible values: auto, on, off]

          --stdin-filename <PATH>
              The name of the file whose text `-` reads from stdin, as an editor passes it: `--jinja auto` decides by it, and messages and diffs show it. The file itself is not read

      -j, --jobs <N>
              Files formatted at once [default: the number of CPUs]. Results are reported in path order whatever the number

      -q, --quiet
              Print only errors, besides the formatted text, the list of files or the diff that is asked for: no summary, warnings or notes

      -v, --verbose
              Also print what happened to each file, and how long it all took

          --color <WHEN>
              When to colour diagnostics; `auto` colours them when stderr is a terminal and `NO_COLOR` is not set

              Possible values:
              - auto:   On a terminal, unless `NO_COLOR` is set to a non-empty value
              - always: Always, also when stderr is a file or a pipe
              - never:  Never

              [default: auto]

      -h, --help
              Print help (see a summary with '-h')

      -V, --version
              Print version

    Exit status: 0 when done, 1 when --check or --diff found files that would change, 2 when a file could not be formatted (unreadable, not UTF-8, a lexer error, a failed verification, or a template refused because its rendering decides where a line comment ends), --verify cannot run clickhouse-format, or the arguments are wrong.
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
    insta::assert_snapshot!(stderr(&output), @"
    chairlift::lex

      × <stdin>:2:8: Single quoted string is not closed; the file is left unchanged
       ╭─[<stdin>:2:8]
     1 │ SELECT 1;
     2 │ SELECT 'x
       ·        ┬
       ·        ╰── here
       ╰────
    ");
    // A template whose line comment the rendering can make run on is left as
    // it is, as one with a lexer error is, and the diagnostic says what to
    // change rather than asking for a bug report.
    let output = run(
        &["--jinja", "on", "-"],
        Some(b"select a -- c\n{%- if x %}\n, b\n{% endif %} from t\n"),
        None,
    );
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    let errors = stderr(&output);
    assert!(
        errors.contains(
            "× <stdin>:1:10: where this line comment ends depends on the rendering, so the file \
             is left unchanged"
        ),
        "{errors}"
    );
    assert!(
        errors.contains("help: the `-` marker of the tag after it strips the line break"),
        "{errors}"
    );
    assert!(!errors.contains("bug"), "{errors}");
    let output = run(&["-"], Some(b"SELECT '\xff'"), None);
    assert_eq!(output.status.code(), Some(2));
    insta::assert_snapshot!(stderr(&output), @"
    chairlift::not_utf8

      × <stdin>:1:9: not UTF-8 (at byte 8); the file is left unchanged
       ╭─[<stdin>:1:9]
     1 │ SELECT '�'
       ·         ┬
       ·         ╰── not UTF-8
       ╰────
    ");
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
    let names: Vec<_> = fs::read_dir(&*dir)
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

/// A file without write permission is not replaced, even where the
/// directory would allow it.
#[cfg(unix)]
#[test]
fn a_read_only_file_is_an_error() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = temp_dir("read-only");
    let file = write(&dir, "ro.sql", MESSY);
    fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();
    let output = run(&["ro.sql"], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    let errors = stderr(&output);
    assert!(errors.contains("× ro.sql: cannot be written: "), "{errors}");
    assert!(
        errors.contains("help: make the file writable, or use --check or --diff"),
        "{errors}"
    );
    assert_eq!(read(&file), MESSY);
    let mode = fs::metadata(&file).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o444);
    // `--check` only reads.
    let output = run(&["--check", "ro.sql"], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));

    // A writable file in a directory that is not.
    let sub = dir.join("ro-dir");
    let file = write(&sub, "rw.sql", MESSY);
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o555)).unwrap();
    let output = run(&["ro-dir/rw.sql"], None, Some(&*dir));
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    let errors = stderr(&output);
    assert!(
        errors.contains("help: the file's directory is not writable"),
        "{errors}"
    );
    assert_eq!(read(&file), MESSY);
}

#[test]
fn check_lists_files_and_writes_nothing() {
    let dir = temp_dir("check");
    let messy = write(&dir, "a/messy.sql", MESSY);
    write(&dir, "a/clean.sql", CLEAN);
    let output = run(&["--check", "a"], None, Some(&*dir));
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
    let output = run(&["--check", "a"], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "");
}

#[test]
fn diff_prints_changes_and_writes_nothing() {
    let dir = temp_dir("diff");
    let messy = write(&dir, "q.sql", "SELECT 1;\n\n\n\nSELECT 2; \n");
    let output = run(&["--diff", "q.sql"], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "--- a/q.sql\n+++ b/q.sql\n@@ -1,5 +1,4 @@\n SELECT 1;\n \n \n-\n-SELECT 2; \n+SELECT 2;\n"
    );
    assert_eq!(read(&messy), "SELECT 1;\n\n\n\nSELECT 2; \n");
    // `--check` with `--diff` diffs.
    let output = run(&["--check", "--diff", "q.sql"], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).starts_with("--- a/q.sql\n"));
}

/// A diff where every line changes takes Myers's algorithm time growing with
/// the square of the lines; past a deadline it is coarser, but still a
/// diff from the input to the output.
#[test]
fn a_diff_of_a_large_file_is_bounded_in_time() {
    let input = "select 1;\n\n".repeat(20_000);
    let started = std::time::Instant::now();
    let output = run(&["--diff", "-"], Some(input.as_bytes()), None);
    assert_eq!(output.status.code(), Some(1));
    assert!(started.elapsed() < std::time::Duration::from_secs(60));
    let diff = stdout(&output);
    assert!(
        diff.starts_with("--- a/-\n+++ b/-\n@@ "),
        "{}",
        &diff[..100]
    );
    assert_eq!(diff.matches("\n-select 1;").count(), 20_000);
    assert_eq!(diff.matches("\n+SELECT 1;").count(), 20_000);
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
    let output = run(&["--check", "."], None, Some(&*dir));
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
        Some(&*dir),
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "Would reformat: ignored.sql\nWould reformat: notes.txt\n"
    );
}

/// A file reached by two spellings, or through a directory and by name, is
/// formatted once, under the first spelling in path order.
#[test]
fn a_file_named_twice_is_formatted_once() {
    let dir = temp_dir("spellings");
    let nested = write(&dir, "d/f1.sql", MESSY);
    let file = write(&dir, "b.sql", MESSY);
    let args = ["d", "./d/f1.sql", "b.sql", "./b.sql", "d/../b.sql"];
    let output = run(&[&["--check"][..], &args].concat(), None, Some(&*dir));
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert_eq!(
        stdout(&output).replace('\\', "/"),
        "Would reformat: ./b.sql\nWould reformat: ./d/f1.sql\n"
    );
    let output = run(&args, None, Some(&*dir));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        "2 files reformatted, 0 files already formatted\n"
    );
    assert_eq!(read(&nested), CLEAN);
    assert_eq!(read(&file), CLEAN);
}

/// The temporary file's name does not grow with the file's, so a file
/// whose name is as long as the file system allows can be written.
#[test]
fn a_file_with_the_longest_name_is_written() {
    let dir = temp_dir("long-name");
    let name = format!("{}.sql", "x".repeat(251));
    let file = write(&dir, &name, MESSY);
    let output = run(&[&name], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(read(&file), CLEAN);
}

#[test]
fn jinja_mode_by_name() {
    let dir = temp_dir("jinja");
    let template = "SELECT '{{ \"'\" }}';\n";
    write(&dir, "t.sql.j2", template);
    write(&dir, "u.j2.sql", template);
    write(&dir, "v.sql", template);
    write(&dir, "W.SQL.J2", template);
    let output = run(
        &["--check", "t.sql.j2", "u.j2.sql", "W.SQL.J2"],
        None,
        Some(&*dir),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let output = run(&["--check", "v.sql"], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(2));
    let output = run(&["--check", "--jinja", "on", "v.sql"], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(0));
    let output = run(
        &["--check", "--jinja", "off", "t.sql.j2"],
        None,
        Some(&*dir),
    );
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn errors_leave_files_alone_and_exit_with_2() {
    let dir = temp_dir("errors");
    let broken = write(&dir, "broken.sql", "SELECT 1;  \nSELECT 'x");
    let latin1 = dir.join("latin1.sql");
    fs::write(&latin1, b"SELECT 'caf\xe9';  \n").unwrap();
    let messy = write(&dir, "messy.sql", MESSY);
    let output = run(&["."], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(read(&broken), "SELECT 1;  \nSELECT 'x");
    assert_eq!(fs::read(&latin1).unwrap(), b"SELECT 'caf\xe9';  \n");
    // The other files are still formatted.
    assert_eq!(read(&messy), CLEAN);
    let errors = stderr(&output).replace('\\', "/");
    insta::assert_snapshot!(errors, @"
    chairlift::lex

      × ./broken.sql:2:8: Single quoted string is not closed; the file is left unchanged
       ╭─[./broken.sql:2:8]
     1 │ SELECT 1;  
     2 │ SELECT 'x
       ·        ┬
       ·        ╰── here
       ╰────
    chairlift::not_utf8

      × ./latin1.sql:1:12: not UTF-8 (at byte 11); the file is left unchanged
       ╭─[./latin1.sql:1:12]
     1 │ SELECT 'caf�';  
       ·            ┬
       ·            ╰── not UTF-8
       ╰────
    1 file reformatted, 0 files already formatted, 2 errors
    ");
    // An error wins over differences found by `--check`.
    fs::write(&messy, MESSY).unwrap();
    assert_eq!(
        run(&["--check", "."], None, Some(&*dir)).status.code(),
        Some(2)
    );
}

/// A refused template is its own outcome in the summary, and the run exits
/// with 2; the other files are handled as usual.
#[test]
fn refused_templates_are_counted_apart() {
    let dir = temp_dir("refused");
    let unsafe_template = "select a -- c\n{%- if x %}\n, b\n{% endif %} from t\n";
    let refused = write(&dir, "m.sql.j2", unsafe_template);
    let messy = write(&dir, "q.sql", MESSY);
    for args in [&["--check", "."][..], &["--diff", "."], &["."]] {
        let output = run(args, None, Some(&*dir));
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let errors = stderr(&output);
        assert!(errors.contains("chairlift::template"), "{errors}");
        assert!(
            errors.ends_with(" already formatted, 1 template refused\n"),
            "{errors}"
        );
        assert_eq!(read(&refused), unsafe_template);
    }
    assert_eq!(read(&messy), CLEAN);
}

#[test]
fn a_missing_path_is_an_error() {
    let dir = temp_dir("missing");
    let output = run(&["nope.sql"], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("× nope.sql: cannot be read: "),
        "{}",
        stderr(&output)
    );
}

/// Files are formatted in parallel, and reported in path order whatever the
/// number of jobs: the list, the diffs and the diagnostics.
#[test]
fn output_is_in_path_order_under_parallelism() {
    let dir = temp_dir("parallel");
    let mut expected = String::new();
    let mut expected_errors = Vec::new();
    for i in 0..60 {
        let name = format!("f{i:02}.sql");
        match i % 3 {
            0 => {
                write(&dir, &name, MESSY);
                expected.extend(["Would reformat: ./", &name, "\n"]);
            }
            1 => {
                write(&dir, &name, CLEAN);
            }
            _ => {
                write(&dir, &name, "SELECT 'x");
                expected_errors.push(format!("./{name}:1:8: Single quoted string"));
            }
        }
    }
    let mut first_diff = None;
    for jobs in ["1", "3", "16"] {
        let output = run(&["--check", "-j", jobs, "."], None, Some(&*dir));
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(stdout(&output).replace('\\', "/"), expected, "-j {jobs}");
        let errors = stderr(&output).replace('\\', "/");
        let mut at = 0;
        for error in &expected_errors {
            let found = errors[at..]
                .find(error.as_str())
                .unwrap_or_else(|| panic!("-j {jobs}: {error} out of order in {errors}"));
            at += found + error.len();
        }
        assert!(
            errors.ends_with(
                "20 files would be reformatted, 20 files already formatted, 20 errors\n"
            ),
            "{errors}"
        );
        let output = run(&["--diff", "--jobs", jobs, "."], None, Some(&*dir));
        let diff = stdout(&output);
        assert_eq!(*first_diff.get_or_insert_with(|| diff.clone()), diff);
    }
}

#[test]
fn select_1_from_stdin() {
    let output = run(&["-"], Some(b"select 1"), None);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "SELECT 1\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn line_width_is_applied() {
    let query = "select f(aaaa, bbbb, cccc), x from tbl where a = 1 and b = 2;";
    let wide = "SELECT\n    f(aaaa, bbbb, cccc),\n    x\nFROM tbl\nWHERE a = 1 AND b = 2;\n";
    let output = run(&["-"], Some(query.as_bytes()), None);
    assert_eq!(stdout(&output), wide);
    let output = run(&["--line-width", "30", "-"], Some(query.as_bytes()), None);
    assert_eq!(stdout(&output), wide);
    let output = run(&["--line-width", "20", "-"], Some(query.as_bytes()), None);
    assert_eq!(
        stdout(&output),
        "SELECT\n    f(\n        aaaa,\n        bbbb,\n        cccc\n    ),\n    x\nFROM tbl\n\
         WHERE a = 1\n    AND b = 2;\n"
    );
    let dir = temp_dir("line-width");
    write(&dir, "q.sql", wide);
    let output = run(
        &["--check", "--line-width", "20", "q.sql"],
        None,
        Some(&*dir),
    );
    assert_eq!(output.status.code(), Some(1));
    for bad in ["0", "-5", "65536", "wide", ""] {
        let output = chairlift(&[&format!("--line-width={bad}"), "-"]);
        assert_eq!(output.status.code(), Some(2), "--line-width {bad:?}");
        assert!(
            stderr(&output).contains("--line-width"),
            "{}",
            stderr(&output)
        );
    }
}

#[test]
fn stdin_filename_decides_jinja_mode_and_names_the_text() {
    // A string holding a tag with a quote in it: one string in Jinja mode,
    // an unclosed string in SQL mode.
    let template = "SELECT '{{ \"'\" }}';\n";
    for name in ["models/m.sql.j2", "0001_init.up.j2.sql"] {
        let output = run(
            &["--stdin-filename", name, "-"],
            Some(template.as_bytes()),
            None,
        );
        assert_eq!(output.status.code(), Some(0), "{name}: {}", stderr(&output));
        assert_eq!(stdout(&output), template);
    }
    let output = run(
        &["--stdin-filename", "models/m.sql", "-"],
        Some(template.as_bytes()),
        None,
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("× models/m.sql:1:14: Double quoted string is not closed"),
        "{}",
        stderr(&output)
    );
    // --jinja still wins.
    let output = run(
        &["--stdin-filename", "m.sql", "--jinja", "on", "-"],
        Some(template.as_bytes()),
        None,
    );
    assert_eq!(output.status.code(), Some(0));

    let output = run(
        &["--check", "--stdin-filename", "q.sql", "-"],
        Some(MESSY.as_bytes()),
        None,
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "Would reformat: q.sql\n");
    let output = run(
        &["--diff", "--stdin-filename", "q.sql", "-"],
        Some(b"SELECT 1;  \n"),
        None,
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).starts_with("--- a/q.sql\n+++ b/q.sql\n"),
        "{}",
        stdout(&output)
    );

    // The file named is not read: it need not exist.
    let output = chairlift(&["--stdin-filename", "q.sql", "x.sql"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--stdin-filename names the file on stdin"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn jobs_must_be_a_positive_number() {
    for bad in ["0", "-1", "many"] {
        let output = chairlift(&["-j", bad, "x.sql"]);
        assert_eq!(output.status.code(), Some(2), "-j {bad}");
    }
}

#[test]
fn quiet_prints_only_errors_and_the_result() {
    let dir = temp_dir("quiet");
    write(&dir, "messy.sql", MESSY);
    write(&dir, "clean.sql", CLEAN);
    let output = run(&["--check", "-q", "."], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output).replace('\\', "/"),
        "Would reformat: ./messy.sql\n"
    );
    assert_eq!(stderr(&output), "");
    write(&dir, "broken.sql", "SELECT 'x");
    let output = run(&["--quiet", "."], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("Single quoted string is not closed"),
        "{}",
        stderr(&output)
    );
    assert!(!stderr(&output).contains("files"), "{}", stderr(&output));
    assert_eq!(read(&dir.join("messy.sql")), CLEAN);
    assert_eq!(
        chairlift(&["-q", "-v", "x.sql"]).status.code(),
        Some(2),
        "--quiet and --verbose conflict"
    );
}

#[test]
fn verbose_says_what_happened_to_each_file() {
    let dir = temp_dir("verbose");
    write(&dir, "messy.sql", MESSY);
    write(&dir, "clean.sql", CLEAN);
    let output = run(&["--check", "-v", "."], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(1));
    let errors = stderr(&output).replace('\\', "/");
    let lines: Vec<&str> = errors.lines().collect();
    assert_eq!(
        lines[..2],
        [
            "./clean.sql: already formatted",
            "./messy.sql: would be reformatted"
        ]
    );
    assert!(
        lines[2].starts_with("1 file would be reformatted, 1 file already formatted in "),
        "{errors}"
    );
    assert!(lines[2].ends_with(" s"), "{errors}");
    let output = run(&["-v", "."], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(0));
    assert!(
        stderr(&output)
            .replace('\\', "/")
            .contains("./messy.sql: reformatted\n"),
        "{}",
        stderr(&output)
    );
}

/// Colour only where asked for: `NO_COLOR` and a pipe turn it off, and
/// `--color always` turns it on.
#[test]
fn colour() {
    let has_escape = |output: &Output| stderr(output).contains('\u{1b}');
    let output = Command::new(env!("CARGO_BIN_EXE_chairlift"))
        .args(["--color", "always", "nope.sql"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(has_escape(&output), "{}", stderr(&output));
    let output = Command::new(env!("CARGO_BIN_EXE_chairlift"))
        .args(["--color", "auto", "nope.sql"])
        .env_remove("NO_COLOR")
        .output()
        .unwrap();
    assert!(
        !has_escape(&output),
        "stderr is a pipe: {}",
        stderr(&output)
    );
    let output = Command::new(env!("CARGO_BIN_EXE_chairlift"))
        .args(["--color", "never", "nope.sql"])
        .output()
        .unwrap();
    assert!(!has_escape(&output), "{}", stderr(&output));
}

/// `clickhouse-format`, as the oracle tests find it: `CHAIRLIFT_CLICKHOUSE_FORMAT`
/// or on `PATH`. `None` skips the test, unless `CHAIRLIFT_REQUIRE_CORPUS` is
/// set.
fn clickhouse_format() -> Option<String> {
    let program = std::env::var("CHAIRLIFT_CLICKHOUSE_FORMAT")
        .unwrap_or_else(|_| "clickhouse-format".to_owned());
    if chairlift::oracle::Oracle::new(&program).is_available() {
        Some(program)
    } else {
        assert!(
            std::env::var_os("CHAIRLIFT_REQUIRE_CORPUS").is_none_or(|v| v.is_empty()),
            "clickhouse-format is unavailable, and CHAIRLIFT_REQUIRE_CORPUS is set"
        );
        eprintln!("skipping: clickhouse-format is unavailable");
        None
    }
}

#[test]
fn verify_needs_clickhouse_format() {
    let dir = temp_dir("verify-missing");
    let messy = write(&dir, "q.sql", MESSY);
    let output = run(
        &[
            "--verify",
            "--clickhouse-binary",
            "/nonexistent/clickhouse-format",
            "q.sql",
        ],
        None,
        Some(&*dir),
    );
    assert_eq!(output.status.code(), Some(2));
    insta::assert_snapshot!(stderr(&output), @"
    chairlift::verify::binary

      × --verify cannot use ClickHouse's formatter: `/nonexistent/clickhouse-format` was not found
      help: --verify runs `clickhouse-format`, which comes with ClickHouse (https://clickhouse.com/docs/install): put it on PATH, or pass --clickhouse-binary with the path to `clickhouse-format` or to the `clickhouse` binary
    ");
    assert_eq!(read(&messy), MESSY, "nothing is written");

    // Not on PATH either.
    let output = Command::new(env!("CARGO_BIN_EXE_chairlift"))
        .args(["--verify", "-"])
        .env("PATH", &*dir)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("`clickhouse-format` was not found"),
        "{}",
        stderr(&output)
    );

    // --clickhouse-binary means nothing without --verify.
    let output = chairlift(&["--clickhouse-binary", "clickhouse-format", "q.sql"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--verify"), "{}", stderr(&output));
}

/// A shell script standing in for `clickhouse-format`.
#[cfg(unix)]
fn fake_oracle(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let path = write(
        dir,
        "bin/clickhouse-format",
        &format!("#!/bin/sh\n{body}\n"),
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A stand-in for `clickhouse-format` that reads two texts the same when they
/// differ only in whitespace: it prints each statement with its whitespace
/// collapsed. `before` runs first, with the input in `$input`.
#[cfg(unix)]
fn whitespace_oracle(dir: &Path, before: &str) -> PathBuf {
    fake_oracle(
        dir,
        &format!(
            r#"input=$(cat)
{before}
printf '%s' "$input" | awk 'BEGIN {{ RS = ";" }}
{{ gsub(/[ \t\n]+/, " "); sub(/^ /, ""); sub(/ $/, ""); if ($0 == "select 1") $0 = "SELECT 1";
   if ($0 != "") print $0 ";" }}'"#
        ),
    )
}

/// A file edited while chairlift runs, here while `--verify` waits for the
/// oracle, is left with the edit, and is an error.
#[cfg(unix)]
#[test]
fn a_file_changed_while_chairlift_runs_is_not_written() {
    let dir = temp_dir("changed-meanwhile");
    let file = write(&dir, "q.sql", "SELECT edit_me   FROM t;\n");
    let other = write(&dir, "r.sql", "SELECT a   FROM t;\n");
    let gone = write(&dir, "s.sql", "SELECT remove_me   FROM t;\n");
    let fake = whitespace_oracle(
        &dir,
        &format!(
            r#"case "$input" in
  *edit_me*) printf 'SELECT edited;\n' > '{}' ;;
esac
case "$input" in
  *remove_me*) rm -f '{}' ;;
esac"#,
            file.display(),
            gone.display()
        ),
    );
    let output = run(
        &[
            "--verify",
            "--clickhouse-binary",
            fake.to_str().unwrap(),
            "q.sql",
            "r.sql",
            "s.sql",
        ],
        None,
        Some(&*dir),
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert_eq!(read(&file), "SELECT edited;\n");
    assert!(!gone.exists());
    assert_eq!(read(&other), "SELECT a\nFROM t;\n");
    let errors = stderr(&output);
    assert!(
        errors.contains(
            "× q.sql: changed while chairlift was running, so it is left as it is; run chairlift \
             again"
        ),
        "{errors}"
    );
    assert!(
        errors.contains(
            "× s.sql: changed while chairlift was running, so it is left as it is; run chairlift \
             again"
        ),
        "{errors}"
    );
    assert!(
        errors.contains("1 file reformatted, 0 files already formatted, 2 errors"),
        "{errors}"
    );
    let names: Vec<_> = fs::read_dir(&*dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 3, "no temporary file is left: {names:?}");
}

#[cfg(unix)]
#[test]
fn verify_refuses_the_wrong_program() {
    let dir = temp_dir("verify-wrong");
    let fake = fake_oracle(&dir, "echo 'Usage: something else'");
    let output = run(
        &[
            "--verify",
            "--clickhouse-binary",
            fake.to_str().unwrap(),
            "-",
        ],
        Some(b"select 1"),
        None,
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains(
            "does not behave like clickhouse-format: given `select 1`, it prints \
             \"Usage: something else\""
        ),
        "{}",
        stderr(&output)
    );
    assert_eq!(stdout(&output), "");
}

/// An oracle that reads the output differently from the input is a bug in
/// chairlift: the file is an error, is not written, and the diagnostic asks
/// for a bug report. The fake oracle passes the check that it formats, and
/// otherwise prints its input, so any change of layout is a difference.
#[cfg(unix)]
#[test]
fn verify_rejects_a_file_the_oracle_reads_differently() {
    let dir = temp_dir("verify-bug");
    let fake = fake_oracle(&dir, "exec sed -e 's/^select 1$/SELECT 1;/'");
    let fake = fake.to_str().unwrap();
    let file = write(&dir, "q.sql", "select a, b from t;\n");
    let clean = write(&dir, "clean.sql", "SELECT 1;\n");
    for args in [
        &[
            "--verify",
            "--clickhouse-binary",
            fake,
            "q.sql",
            "clean.sql",
        ][..],
        &[
            "--verify",
            "--check",
            "--clickhouse-binary",
            fake,
            "q.sql",
            "clean.sql",
        ],
        &[
            "--verify",
            "--diff",
            "--clickhouse-binary",
            fake,
            "q.sql",
            "clean.sql",
        ],
    ] {
        let output = run(args, None, Some(&*dir));
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(stdout(&output), "", "{args:?}");
        assert_eq!(read(&file), "select a, b from t;\n");
        assert_eq!(read(&clean), "SELECT 1;\n");
    }
    let output = run(
        &["--verify", "--clickhouse-binary", fake, "q.sql"],
        None,
        Some(&*dir),
    );
    insta::assert_snapshot!(stderr(&output), @r#"
    chairlift::verify::clickhouse

      × q.sql: clickhouse-format reads the formatted text differently at statement 1 (line 1, line 1 of the output), so the file is left unchanged
       ╭─[q.sql:1:1]
     1 │ select a, b from t;
       · ┬
       · ╰── this statement
       ╰────
      help: the oracle prints "select a, b from t;" for the input and "SELECT\n    a,\n    b\nFROM t;" for the output
            this is a bug in chairlift. Please report it at https://github.com/glennib/chairlift/issues with the file, or the smallest part of it that shows the problem, and the output of `chairlift --version`.
    0 files reformatted, 0 files already formatted, 1 error
    clickhouse-format reads 0 of 1 statements in changed files the same
    "#);
    let output = run(
        &["--verify", "--clickhouse-binary", fake, "-"],
        Some(b"select a, b from t;\n"),
        None,
    );
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("<stdin>: clickhouse-format reads the formatted text differently"),
        "{}",
        stderr(&output)
    );
}

/// An oracle that crashes, or prints without end, on everything but the
/// check that it formats judges nothing: each changed file is an error and
/// is left as it is, also with `--quiet`.
#[cfg(unix)]
#[test]
fn verify_fails_a_file_the_oracle_cannot_judge() {
    let dir = temp_dir("verify-crash");
    let file = write(&dir, "q.sql", "select a from t;\n");
    let clean = write(&dir, "clean.sql", "SELECT 1;\n");
    for misbehaviour in ["kill -SEGV $$", "exec yes SELECT"] {
        let fake = fake_oracle(
            &dir,
            &format!(
                r#"input=$(cat)
if [ "$input" = "select 1" ]; then echo 'SELECT 1;'; else {misbehaviour}; fi"#
            ),
        );
        let fake = fake.to_str().unwrap();
        let output = run(
            &[
                "--verify",
                "--check",
                "--clickhouse-binary",
                fake,
                "q.sql",
                "clean.sql",
            ],
            None,
            Some(&*dir),
        );
        assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
        assert_eq!(stdout(&output), "");
        assert_eq!(
            stderr(&output),
            "chairlift::verify::failed\n\n  × q.sql: clickhouse-format judged none of its \
             statements, as it failed (crashed, timed out or printed too much) on 1, so the file \
             is left unchanged\n\
             0 files would be reformatted, 1 file already formatted, 1 error\n\
             clickhouse-format reads 0 of 1 statements in changed files the same; not judged: 1 \
             on which it failed\n"
        );
        for (path, stdin) in [("q.sql", None), ("-", Some(&b"select a from t;\n"[..]))] {
            let args = ["--verify", "--quiet", "--clickhouse-binary", fake, path];
            let output = run(&args, stdin, Some(&*dir));
            assert_eq!(output.status.code(), Some(2), "{args:?}");
            assert!(
                stderr(&output).contains("judged none of its statements"),
                "{args:?}: {}",
                stderr(&output)
            );
            assert_eq!(stdout(&output), "", "{args:?}");
        }
        assert_eq!(read(&file), "select a from t;\n");
        assert_eq!(read(&clean), "SELECT 1;\n");
    }
}

/// Whether the process `pid` has gone, or is a zombie waiting to be
/// reaped; waits a little for it.
#[cfg(target_os = "linux")]
fn is_gone(pid: &str) -> bool {
    let stat = format!("/proc/{pid}/stat");
    for _ in 0..100 {
        match fs::read_to_string(&stat) {
            Err(_) => return true,
            Ok(s)
                if s.rsplit(')')
                    .next()
                    .is_some_and(|r| r.trim_start().starts_with('Z')) =>
            {
                return true;
            }
            Ok(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    false
}

/// An oracle that hangs on everything but the check that it formats, and
/// leaves a process of its own hanging: after a few timeouts `--verify`
/// stops running it, every file it did not judge is an error and is left
/// as it is, and no process it started is left behind.
#[cfg(target_os = "linux")]
#[test]
fn verify_gives_up_on_an_oracle_that_hangs() {
    let dir = temp_dir("verify-hang");
    let pids = dir.join("pids");
    let fake = fake_oracle(
        &dir,
        &format!(
            r#"input=$(cat)
if [ "$input" = "select 1" ]; then echo 'SELECT 1;'; exit; fi
(sleep 1000 & echo $! >> '{}'); sleep 1000"#,
            pids.display()
        ),
    );
    let fake = fake.to_str().unwrap();
    let files: Vec<PathBuf> = (1..=4)
        .map(|n| {
            write(
                &dir,
                &format!("q{n}.sql"),
                &format!("select {n};\nselect a;\n"),
            )
        })
        .collect();
    let started = std::time::Instant::now();
    let output = run(
        &[
            "--verify",
            "--verify-timeout",
            "0.3",
            "-j",
            "1",
            "--clickhouse-binary",
            fake,
            ".",
        ],
        None,
        Some(&*dir),
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    let errors = stderr(&output);
    assert!(
        errors.contains(
            "× clickhouse-format timed out 3 times in a row, so --verify stopped running it"
        ),
        "{errors}"
    );
    assert_eq!(errors.matches("× ./q").count(), 4, "{errors}");
    for (n, file) in files.iter().enumerate() {
        assert_eq!(read(file), format!("select {};\nselect a;\n", n + 1));
    }
    for pid in read(&pids).lines() {
        assert!(is_gone(pid), "process {pid} is still running");
    }
}

/// `SIGTERM` stops chairlift and the oracle processes it is waiting for,
/// with what they started.
#[cfg(target_os = "linux")]
#[test]
fn a_signal_stops_the_oracle_too() {
    let dir = temp_dir("verify-signal");
    let pids = dir.join("pids");
    let fake = fake_oracle(
        &dir,
        &format!(
            r#"input=$(cat)
if [ "$input" = "select 1" ]; then echo 'SELECT 1;'; exit; fi
echo $$ >> '{0}'; (sleep 1000 & echo $! >> '{0}'); sleep 1000"#,
            pids.display()
        ),
    );
    let file = write(&dir, "q.sql", "select a;\n");
    let mut child = Command::new(env!("CARGO_BIN_EXE_chairlift"))
        .args([
            "--verify",
            "-j",
            "1",
            "--clickhouse-binary",
            fake.to_str().unwrap(),
        ])
        .arg(&file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Both the oracle and its own process have started.
    for _ in 0..500 {
        if fs::read_to_string(&pids).is_ok_and(|p| p.lines().count() >= 2) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let status = child.wait().unwrap();
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&status),
        Some(15)
    );
    let pids = read(&pids);
    assert!(pids.lines().count() >= 2, "{pids}");
    for pid in pids.lines() {
        assert!(is_gone(pid), "process {pid} is still running");
    }
    assert_eq!(read(&file), "select a;\n");
}

/// An oracle that crashes on some statements of a file and judges others:
/// a warning that `--quiet` does not hide, and the file is written.
#[cfg(unix)]
#[test]
fn verify_warns_when_the_oracle_crashes_on_a_statement() {
    let dir = temp_dir("verify-crash-some");
    let fake = whitespace_oracle(
        &dir,
        r#"case "$input" in *crash_me*) kill -SEGV $$ ;; esac"#,
    );
    let fake = fake.to_str().unwrap();
    let file = write(
        &dir,
        "q.sql",
        "SELECT a   FROM t;\nSELECT crash_me   FROM t;\n",
    );
    let output = run(
        &["--verify", "--check", "--clickhouse-binary", fake, "q.sql"],
        None,
        Some(&*dir),
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert_eq!(stdout(&output), "Would reformat: q.sql\n");
    assert_eq!(
        stderr(&output),
        "chairlift::verify::failed\n\n  ⚠ q.sql: clickhouse-format failed (crashed, timed out \
         or printed too much) on 1 statement, not judged\n\
         1 file would be reformatted, 0 files already formatted\n\
         clickhouse-format reads 1 of 2 statements in changed files the same; not judged: 1 \
         on which it failed\n"
    );
    let output = run(
        &["--verify", "--quiet", "--clickhouse-binary", fake, "q.sql"],
        None,
        Some(&*dir),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        "chairlift::verify::failed\n\n  ⚠ q.sql: clickhouse-format failed (crashed, timed out \
         or printed too much) on 1 statement, not judged\n"
    );
    assert_eq!(
        read(&file),
        "SELECT a\nFROM t;\n\nSELECT crash_me\nFROM t;\n"
    );
}

#[test]
fn verify_with_clickhouse_format() {
    let Some(oracle) = clickhouse_format() else {
        return;
    };
    let dir = temp_dir("verify");
    let messy = write(&dir, "messy.sql", "select a, b from t;\nselect (;\n");
    write(&dir, "clean.sql", CLEAN);
    let template = write(
        &dir,
        "model.sql.j2",
        "{{ config(x=1) }}\nselect a from {{ ref('t') }};\nselect   2;\n",
    );
    let args = ["--verify", "--clickhouse-binary", &oracle, "-j", "2"];

    let output = run(&[&args[..], &["--check", "."]].concat(), None, Some(&*dir));
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert_eq!(
        stdout(&output).replace('\\', "/"),
        "Would reformat: ./messy.sql\nWould reformat: ./model.sql.j2\n"
    );
    // With the `template` feature the template is judged rendered as well:
    // its two statements, once.
    let expected = if cfg!(feature = "template") {
        "2 files would be reformatted, 1 file already formatted\n\
         clickhouse-format reads 4 of 6 statements in changed files the same; not judged: \
         1 with Jinja, 1 it cannot parse\n"
    } else {
        "2 files would be reformatted, 1 file already formatted\n\
         clickhouse-format reads 2 of 4 statements in changed files the same; not judged: \
         1 with Jinja, 1 it cannot parse; 1 template not rendered (chairlift is built without \
         the `template` feature)\n"
    };
    assert_eq!(stderr(&output), expected);

    let output = run(
        &[&args[..], &["-v", "--diff", "."]].concat(),
        None,
        Some(&*dir),
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stdout(&output).contains("+SELECT\n+    a,\n+    b\n+FROM t;\n"));
    assert_eq!(
        stderr(&output).contains(
            "model.sql.j2: a template, judged without rendering it: 1 statement with Jinja \
             not judged"
        ),
        !cfg!(feature = "template"),
        "{}",
        stderr(&output)
    );

    let output = run(&[&args[..], &["."]].concat(), None, Some(&*dir));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        read(&messy),
        "SELECT\n    a,\n    b\nFROM t;\n\nSELECT (;\n"
    );
    assert!(read(&template).ends_with("\nSELECT 2;\n"));

    let output = run(&[&args[..], &["-"]].concat(), Some(b"select 1"), None);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "SELECT 1\n");
    assert_eq!(stderr(&output), "");

    // clickhouse-format takes the rest of the line after `VALUES` data as
    // data, so the whole texts differ while each statement reads the same:
    // a warning, not an error. It shows nothing of what the oracle printed,
    // as that holds the secrets `--show_secrets` reveals.
    let output = run(
        &[&args[..], &["-"]].concat(),
        Some(
            b"INSERT INTO t VALUES (1); select 2; \
              create user u identified with plaintext_password by 'hunter2';\n",
        ),
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        stdout(&output).starts_with("INSERT INTO t VALUES (1);\nSELECT 2;\n"),
        "{}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains(
            "⚠ <stdin>: clickhouse-format reads the formatted text differently as a whole"
        ),
        "{}",
        stderr(&output)
    );
    assert!(!stderr(&output).contains("hunter2"), "{}", stderr(&output));

    // The single `clickhouse` binary formats as `clickhouse format`.
    let clickhouse = Path::new(&oracle).with_file_name("clickhouse");
    if clickhouse.is_file() {
        let output = run(
            &[
                "--verify",
                "--clickhouse-binary",
                clickhouse.to_str().unwrap(),
                "-",
            ],
            Some(b"select 1"),
            None,
        );
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert_eq!(stdout(&output), "SELECT 1\n");
    }
}
