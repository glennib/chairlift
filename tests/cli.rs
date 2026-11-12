//! End-to-end tests of the `chl` binary.

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
    let mut command = Command::new(env!("CARGO_BIN_EXE_chl"));
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
fn version_names_the_keyword_list_release() {
    let first = format!(
        "chairlift {} (ClickHouse keywords {})\n",
        env!("CARGO_PKG_VERSION"),
        chairlift::KEYWORDS_VERSION
    );
    let output = chairlift(&["-V"]);
    assert!(output.status.success());
    assert_eq!(stdout(&output), first);
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

    Files are formatted in place. Directories are searched for `*.sql`, `*.sql.j2` and `*.j2.sql` files, skipping hidden files, symbolic links and whatever `.gitignore` and `.ignore` files exclude. A file named on the command line is formatted whatever its name.

    Usage: chl [OPTIONS] <PATHS>...

    Arguments:
      <PATHS>...
              Files or directories to format; `-` reads stdin and writes stdout

    Options:
          --check
              Write nothing; list the files that would change and exit with 1 if there are any

          --diff
              Write nothing; print a unified diff of the changes and exit with 1 if there are any. With --check too, the diff is printed instead of the list

          --line-width <WIDTH>
              The width to lay lines out to

              [default: 100]

          --jinja <WHEN>
              Whether files are Jinja templates; `auto` decides by name (`*.j2`, such as `*.sql.j2`, and `*.j2.sql` are), and stdin is not unless --stdin-filename names a template

              [default: auto]
              [possible values: auto, on, off]

          --stdin-filename <PATH>
              The name of the file whose text `-` reads from stdin, as an editor passes it: `--jinja auto` decides by it, and messages and diffs show it. The file itself is not read

      -j, --jobs <N>
              Files formatted at once [default: the number of CPUs]. Results are reported in path order whatever the number

      -q, --quiet
              Print only errors, besides the formatted text, the list of files or the diff that is asked for: no summary or warnings

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

    Exit status: 0 when done, 1 when --check or --diff found files that would change, 2 when a file could not be formatted (unreadable, not UTF-8, a lexer error, a failed verification, or a template refused because its rendering decides where a line comment ends) or the arguments are wrong.
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

/// A directory with no SQL file in it is worth a warning, which does not
/// change the exit status.
#[test]
fn a_directory_without_sql_files_is_warned_about() {
    let dir = temp_dir("no-sql");
    write(&dir, "empty/notes.txt", "select 1");
    write(&dir, "full/a.sql", "SELECT 1;\n");
    let output = run(&["--check", "empty", "full"], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        "chairlift::walk::empty\n\n  ⚠ empty: no SQL files found in this directory (`*.sql`, \
         `*.sql.j2`, `*.j2.sql`, not hidden or ignored)\n\
         0 files would be reformatted, 1 file already formatted\n"
    );
    let output = run(&["--check", "-q", "empty"], None, Some(&*dir));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stderr(&output), "");
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

/// A reader of stdout or stderr that stops reading, as `head` does, stops
/// the output quietly: no panic, no error, and the exit status the run
/// would have had.
#[test]
fn a_closed_pipe_stops_the_output_quietly() {
    let dir = temp_dir("pipe");
    for n in 0..3000 {
        write(&dir, &format!("f{n}.sql"), "select 1");
    }
    for (args, closed) in [(&["--check"][..], "stdout"), (&["--check", "-v"], "stderr")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_chl"));
        command.args(args).arg(&*dir).stdin(Stdio::null());
        if closed == "stdout" {
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
        } else {
            command.stdout(Stdio::null()).stderr(Stdio::piped());
        }
        let mut child = command.spawn().unwrap();
        // More than a pipe holds goes to the closed pipe.
        let mut stderr = String::new();
        if closed == "stdout" {
            drop(child.stdout.take());
            std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
        } else {
            drop(child.stderr.take());
        }
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(1), "{closed}: {stderr}");
        assert_eq!(
            stderr,
            if closed == "stdout" {
                "3000 files would be reformatted, 0 files already formatted\n"
            } else {
                ""
            }
        );
    }
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
    let output = Command::new(env!("CARGO_BIN_EXE_chl"))
        .args(["--color", "always", "nope.sql"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(has_escape(&output), "{}", stderr(&output));
    let output = Command::new(env!("CARGO_BIN_EXE_chl"))
        .args(["--color", "auto", "nope.sql"])
        .env_remove("NO_COLOR")
        .output()
        .unwrap();
    assert!(
        !has_escape(&output),
        "stderr is a pipe: {}",
        stderr(&output)
    );
    let output = Command::new(env!("CARGO_BIN_EXE_chl"))
        .args(["--color", "never", "nope.sql"])
        .output()
        .unwrap();
    assert!(!has_escape(&output), "{}", stderr(&output));
}
