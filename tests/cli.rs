//! End-to-end tests of the `chl` binary.

use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

fn chairlift(args: &[&str]) -> Output {
    run(args, None, None)
}

/// Runs `chl` in `dir`, or in a fresh directory of its own.
fn run(args: &[&str], stdin: Option<&[u8]>, dir: Option<&Path>) -> Output {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let own;
    let dir = if let Some(dir) = dir {
        dir
    } else {
        own = temp_dir(&format!("run-{}", RUNS.fetch_add(1, Ordering::Relaxed)));
        &*own
    };
    let mut command = chl(dir);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("chairlift binary runs");
    let mut pipe = child.stdin.take().unwrap();
    if let Some(input) = stdin {
        // `chl` can exit without reading stdin, on a bad `chl.toml` for one.
        match pipe.write_all(input) {
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
            result => result.unwrap(),
        }
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

/// The `chl` binary, to run in `dir`, which is a [`temp_dir`] or in one:
/// `chl` reads the nearest `chl.toml` above its paths, or above the
/// directory it runs in for stdin without --stdin-filename.
fn chl(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_chl"));
    command.current_dir(dir);
    command
}

/// A fresh directory for a test, outside any repository so that no
/// `.gitignore` above it applies, with an empty `chl.toml` so that none
/// above it does; removed when dropped.
fn temp_dir(test: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!("chairlift-cli-{}-{test}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("chl.toml"), "").unwrap();
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
/// [`WIDE`] at the default width, [`NARROW`] at 20.
const QUERY: &[u8] = b"select f(aaaa, bbbb, cccc), x from tbl where a = 1 and b = 2;";
const WIDE: &str = "SELECT\n    f(aaaa, bbbb, cccc),\n    x\nFROM tbl\nWHERE a = 1 AND b = 2;\n";
const NARROW: &str = "SELECT\n    f(\n        aaaa,\n        bbbb,\n        cccc\n    ),\n    x\nFROM \
                      tbl\nWHERE a = 1\n    AND b = 2;\n";
/// Lexes only as a template.
const TEMPLATE: &[u8] = b"SELECT '{{ \"'\" }}';\n";

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

    Files are formatted in place. Directories are searched for `*.sql`, `*.sql.j2` and `*.j2.sql` files, skipping hidden files, symbolic links and whatever `.gitignore`, `.ignore` and `.chlignore` files exclude; `.chlignore` excludes files from formatting only. A file named on the command line is formatted whatever its name, unless `.chlignore`, `chl.toml` or --exclude excludes it.

    A `chl.toml` can set `line-width`, `jinja` and `exclude`; a flag given on the command line overrides its value. The one read is the nearest one in or above the deepest directory that holds all the paths; for stdin, in or above the directory of the --stdin-filename name, or the current directory without one.

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
              The name of the file whose text `-` reads from stdin, as an editor passes it: `--jinja auto` decides by it, and messages and diffs show it. The file itself is not read. When `.chlignore`, `chl.toml` or --exclude excludes it, the text is not formatted: it is written back as it is, and --check and --diff find nothing to change

          --exclude <GLOB>
              Skip the files and directories that GLOB matches, named or in searched directories, in gitignore syntax relative to the current directory, besides those `chl.toml` excludes; may be given more than once, and `!GLOB` brings back what an earlier --exclude skipped

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

    Exit status: 0 when done, 1 when --check or --diff found files that would change, 2 when a file could not be formatted (unreadable, not UTF-8, a lexer error, a failed verification, or a template refused because its rendering decides where a line comment ends) or the arguments or `chl.toml` are wrong.
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
    let mut names: Vec<_> = fs::read_dir(&*dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names, ["chl.toml", "clean.sql", "messy.sql"]);
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

/// The files `chl --check` lists, run in `dir` on `args`, with `/` as the
/// separator.
fn listed(dir: &Path, args: &[&str]) -> Vec<String> {
    let output = run(&[&["--check"][..], args].concat(), None, Some(dir));
    assert_ne!(output.status.code(), Some(2), "{}", stderr(&output));
    stdout(&output)
        .lines()
        .map(|l| {
            l.strip_prefix("Would reformat: ")
                .unwrap()
                .replace('\\', "/")
        })
        .collect()
}

#[test]
fn chlignore_excludes_files_from_formatting() {
    for in_git in [false, true] {
        let dir = temp_dir(&format!("chlignore-{in_git}"));
        if in_git {
            fs::create_dir(dir.join(".git")).unwrap();
        }
        for name in [
            "a.sql",
            "generated.sql",
            "vendor/b.sql",
            "sub/c.sql",
            "sub/nested.sql",
        ] {
            write(&dir, name, "SELECT 1; ");
        }
        write(&dir, ".chlignore", "generated.sql\nvendor/\n");
        write(&dir, "sub/.chlignore", "nested.sql\n");
        assert_eq!(listed(&dir, &["."]), ["./a.sql", "./sub/c.sql"], "{in_git}");
        assert_eq!(
            listed(&dir, &["a.sql", "generated.sql", "vendor/b.sql", "vendor"]),
            ["a.sql"],
            "{in_git}"
        );
    }
}

/// `.chlignore` wins over `.gitignore` and `.ignore` either way: it can
/// bring back a file they exclude, and they cannot bring back one it
/// excludes. A directory they exclude is not searched, so `.chlignore`
/// cannot bring back the files in it.
#[test]
fn chlignore_wins_over_gitignore_and_ignore() {
    let dir = temp_dir("chlignore-precedence");
    fs::create_dir(dir.join(".git")).unwrap();
    for name in [
        "back-from-git.sql",
        "back-from-ignore.sql",
        "kept-out.sql",
        "skipped/d.sql",
    ] {
        write(&dir, name, "SELECT 1; ");
    }
    write(
        &dir,
        ".gitignore",
        "back-from-git.sql\n!kept-out.sql\nskipped/\n",
    );
    write(&dir, ".ignore", "back-from-ignore.sql\n!kept-out.sql\n");
    write(
        &dir,
        ".chlignore",
        "!back-from-git.sql\n!back-from-ignore.sql\nkept-out.sql\n!skipped/d.sql\n",
    );
    assert_eq!(
        listed(&dir, &["."]),
        ["./back-from-git.sql", "./back-from-ignore.sql"]
    );
}

/// A `.chlignore` in a directory above the one searched applies, whether
/// the directory is given by a relative path or an absolute one, with its
/// patterns relative to its own directory.
#[test]
fn chlignore_in_a_parent_directory_applies() {
    let dir = temp_dir("chlignore-parents");
    for name in ["sub/a.sql", "sub/anywhere.sql", "sub/anchored.sql"] {
        write(&dir, name, "SELECT 1; ");
    }
    write(&dir, ".chlignore", "anywhere.sql\n/sub/anchored.sql\n");
    assert_eq!(listed(&dir, &["sub"]), ["sub/a.sql"]);
    assert_eq!(listed(&dir.join("sub"), &["."]), ["./a.sql"]);
    let sub = dir.join("sub");
    let sub = sub.to_str().unwrap();
    assert_eq!(
        listed(&dir, &[sub]),
        [format!("{}/a.sql", sub.replace('\\', "/"))]
    );
}

/// A `.chlignore` line that cannot be parsed is an error wherever the file
/// lies: in or below a searched directory, above one, or above a named
/// path. Each file is reported once, the rest of it still applies, and the
/// other files are still formatted.
#[test]
fn an_unparsable_chlignore_line_is_reported_once() {
    let dir = temp_dir("chlignore-unparsable");
    write(&dir, "d/.chlignore", "{\nskipped.sql\n");
    write(&dir, "d/e/.chlignore", "{\n");
    for name in ["c.sql", "d/a.sql", "d/skipped.sql", "d/e/b.sql"] {
        write(&dir, name, MESSY);
    }
    let runs: [(&[&str], &str); 7] = [
        (&["d"], "d/a.sql d/e/b.sql"),
        (&["."], "./c.sql ./d/a.sql ./d/e/b.sql"),
        (&["d", "d/a.sql"], "d/a.sql d/e/b.sql"),
        (&["d/a.sql", "d/skipped.sql", "d"], "d/a.sql d/e/b.sql"),
        (&["d/e", "d"], "d/a.sql d/e/b.sql"),
        (&["d/e/b.sql", "d/e"], "d/e/b.sql"),
        (&["d", "d"], "d/a.sql d/e/b.sql"),
    ];
    for (paths, files) in runs {
        let args = [&["--check"][..], paths].concat();
        let output = run(&args, None, Some(&dir));
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let errors = stderr(&output).replace('\\', "/");
        assert_eq!(
            errors.matches("chairlift::ignore_file").count(),
            2,
            "{args:?}: {errors}"
        );
        for file in ["/d/.chlignore: line 1: ", "/d/e/.chlignore: line 1: "] {
            assert_eq!(errors.matches(file).count(), 1, "{args:?}: {errors}");
        }
        assert!(!errors.contains("chairlift::walk"), "{args:?}: {errors}");
        let listed: Vec<String> = stdout(&output)
            .lines()
            .map(|l| {
                l.strip_prefix("Would reformat: ")
                    .unwrap()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(listed.join(" "), files, "{args:?}");
    }

    let output = run(&["d"], None, Some(&dir));
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(read(&dir.join("d/a.sql")), CLEAN);
    assert_eq!(read(&dir.join("d/e/b.sql")), CLEAN);
    assert_eq!(read(&dir.join("d/skipped.sql")), MESSY);
}

/// `--exclude` globs are relative to the current directory whichever way
/// the searched directory is spelled, and an excluded directory is not
/// searched.
#[test]
fn exclude_skips_files_and_directories() {
    let dir = temp_dir("exclude");
    for name in [
        "a.sql",
        "b.gen.sql",
        "models/c.sql",
        "models/d.gen.sql",
        "models/vendor/e.sql",
        "models/vendor/deep/f.sql",
    ] {
        write(&dir, name, "SELECT 1; ");
    }
    let vendor = ["--exclude", "models/vendor"];
    assert_eq!(
        listed(&dir, &[&vendor[..], &["models"]].concat()),
        ["models/c.sql", "models/d.gen.sql"]
    );
    assert_eq!(
        listed(&dir, &[&vendor[..], &["."]].concat()),
        [
            "./a.sql",
            "./b.gen.sql",
            "./models/c.sql",
            "./models/d.gen.sql"
        ]
    );
    let models = dir.join("models");
    let shown = models.to_str().unwrap().replace('\\', "/");
    assert_eq!(
        listed(&dir, &[&vendor[..], &[models.to_str().unwrap()]].concat()),
        [format!("{shown}/c.sql"), format!("{shown}/d.gen.sql")]
    );
    assert_eq!(
        listed(&dir, &["--exclude", "*.gen.sql", "--exclude", "deep/", "."]),
        ["./a.sql", "./models/c.sql", "./models/vendor/e.sql"]
    );
    // Anchored to the current directory, not to the searched one.
    assert_eq!(listed(&dir, &["--exclude", "/c.sql", "models"]).len(), 4);
    assert_eq!(
        listed(&dir, &["--exclude", "/models/c.sql", "models"]).len(),
        3
    );
    assert_eq!(
        listed(&models, &["--exclude", "/vendor", "."]),
        ["./c.sql", "./d.gen.sql"]
    );
    assert_eq!(
        listed(&dir, &["--exclude", "*.gen.sql", "a.sql", "b.gen.sql"]),
        ["a.sql"]
    );
}

/// `!GLOB` brings back what an earlier `--exclude` skipped, as a `!` line
/// does in a `.gitignore`, but not what an ignore file excludes.
#[test]
fn exclude_negation_undoes_only_an_earlier_exclude() {
    let dir = temp_dir("exclude-negation");
    for name in ["a.gen.sql", "keep.gen.sql", "ignored.sql", "chlignored.sql"] {
        write(&dir, name, "SELECT 1; ");
    }
    write(&dir, ".ignore", "ignored.sql\n");
    write(&dir, ".chlignore", "chlignored.sql\n");
    let args = [
        "--exclude",
        "*.gen.sql",
        "--exclude",
        "!keep.gen.sql",
        "--exclude",
        "!ignored.sql",
        "--exclude",
        "!chlignored.sql",
        ".",
    ];
    assert_eq!(listed(&dir, &args), ["./keep.gen.sql"]);
    // The last glob that matches decides.
    let args = ["--exclude", "!keep.gen.sql", "--exclude", "*.gen.sql", "."];
    assert_eq!(listed(&dir, &args), Vec::<String>::new());
}

#[test]
fn an_invalid_exclude_glob_exits_with_2() {
    let output = chairlift(&["--exclude", "a[b", "x.sql"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("invalid --exclude: error parsing glob 'a[b'"),
        "{}",
        stderr(&output)
    );
}

/// A tree whose `.chlignore` files and `--exclude` globs exercise nesting,
/// directory patterns and `!` negations, with the SQL files a walk of it
/// formats.
fn excluded_tree(test: &str) -> (TempDir, Vec<&'static str>, Vec<&'static str>) {
    let dir = temp_dir(test);
    let all = vec![
        "a.sql",
        "a.skip.sql",
        "deep/a/b/c.sql",
        "deep/a/bb/c.sql",
        "docs/vendor.sql",
        "gen/g.sql",
        "gen/keep.sql",
        "keep.gen.sql",
        "local.sql",
        "other/vendor/o.sql",
        "out/keep.sql",
        "out/o.sql",
        "sub/a.skip.sql",
        "sub/keep.gen.sql",
        "sub/local.sql",
        "sub/top.sql",
        "sub/vendor/w.sql",
        "sub/x.tmp.sql",
        "sub/y.tmp.sql",
        "top.sql",
        "vendor/v.sql",
        "x.tmp.sql",
        "z.gen.sql",
    ];
    for name in &all {
        write(&dir, name, "SELECT 1; ");
    }
    write(
        &dir,
        ".chlignore",
        "gen/\n!gen/keep.sql\n*.tmp.sql\nvendor\n/top.sql\n*.gen.sql\n!keep.gen.sql\n",
    );
    write(&dir, "sub/.chlignore", "!y.tmp.sql\nlocal.sql\n");
    write(&dir, "other/.chlignore", "!vendor/\n");
    write(&dir, "deep/a/.chlignore", "b/\n");
    let formatted = vec![
        "a.sql",
        "deep/a/bb/c.sql",
        "docs/vendor.sql",
        "keep.gen.sql",
        "local.sql",
        "other/vendor/o.sql",
        "sub/a.skip.sql",
        "sub/keep.gen.sql",
        "sub/top.sql",
        "sub/y.tmp.sql",
    ];
    (dir, all, formatted)
}

const EXCLUDES: [&str; 8] = [
    "--exclude",
    "out/",
    "--exclude",
    "!out/keep.sql",
    "--exclude",
    "*.skip.sql",
    "--exclude",
    "!sub/*.skip.sql",
];

/// A file named on the command line is excluded exactly when a walk of a
/// directory above it would leave it out, by name or by its directory.
#[test]
fn a_named_file_is_excluded_as_the_walk_excludes_it() {
    let (dir, all, formatted) = excluded_tree("named-as-walked");
    let walked = listed(&dir, &[&EXCLUDES[..], &["."]].concat());
    let walked: Vec<&str> = walked.iter().map(|f| &f[2..]).collect();
    assert_eq!(walked, formatted);
    assert_eq!(listed(&dir, &[&EXCLUDES[..], &all].concat()), formatted);
    let absolute: Vec<String> = all
        .iter()
        .map(|f| dir.join(f).to_str().unwrap().to_owned())
        .collect();
    let absolute: Vec<&str> = absolute.iter().map(String::as_str).collect();
    let prefix = format!("{}/", dir.to_str().unwrap().replace('\\', "/"));
    let named = listed(&dir, &[&EXCLUDES[..], &absolute].concat());
    let named: Vec<&str> = named
        .iter()
        .map(|f| f.strip_prefix(&prefix).unwrap())
        .collect();
    assert_eq!(named, formatted);
}

/// A named directory that is excluded, or is in an excluded directory, is
/// not searched; it is warned about, as an empty one is.
#[test]
fn a_named_directory_is_excluded_as_the_walk_excludes_it() {
    let (dir, _, _) = excluded_tree("named-directories");
    let excluded = ["gen", "deep/a/b", "sub/vendor", "out", "vendor", "gen/."];
    let output = run(
        &[&["--check"][..], &EXCLUDES, &excluded].concat(),
        None,
        Some(&*dir),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    let errors = stderr(&output).replace('\\', "/");
    for name in excluded {
        assert!(
            errors.contains(&format!("⚠ {name}: not searched, excluded by ")),
            "{name}: {errors}"
        );
    }
    assert!(errors.contains("⚠ out: not searched, excluded by --exclude `out/`\n"));
    assert!(!errors.contains("no SQL files found"), "{errors}");
    let output = run(
        &[&["--check", "-q"][..], &EXCLUDES, &excluded].concat(),
        None,
        Some(&*dir),
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stderr(&output), "");

    assert_eq!(listed(&dir, &["other/vendor"]), ["other/vendor/o.sql"]);
    // The `.chlignore` files above the current directory apply to it.
    assert_eq!(listed(&dir.join("vendor"), &["."]), Vec::<String>::new());
    assert_eq!(
        listed(&dir.join("gen"), &["keep.sql", "../a.sql"]),
        ["../a.sql"]
    );
}

/// pre-commit and lint-staged name each staged file: those that are
/// excluded are left alone without a word, and if all are, the run
/// succeeds.
#[test]
fn excluded_named_files_are_skipped_quietly() {
    let dir = temp_dir("named-excluded");
    for name in ["a.sql", "vendor/b.sql", "c.gen.sql"] {
        write(&dir, name, MESSY);
    }
    write(&dir, ".chlignore", "vendor/\n");
    let args = [
        "--exclude",
        "*.gen.sql",
        "a.sql",
        "vendor/b.sql",
        "c.gen.sql",
    ];
    let output = run(&args, None, Some(&*dir));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        "1 file reformatted, 0 files already formatted\n"
    );
    assert_eq!(read(&dir.join("a.sql")), CLEAN);
    assert_eq!(read(&dir.join("vendor/b.sql")), MESSY);
    assert_eq!(read(&dir.join("c.gen.sql")), MESSY);

    let args = ["--exclude", "*.gen.sql", "vendor/b.sql", "c.gen.sql"];
    for action in [None, Some("--check"), Some("--diff")] {
        let output = run(&[action.as_slice(), &args].concat(), None, Some(&*dir));
        assert_eq!(output.status.code(), Some(0), "{action:?}");
        assert_eq!(stdout(&output), "", "{action:?}");
        let summary = if action.is_some() {
            "0 files would be reformatted, 0 files already formatted\n"
        } else {
            "0 files reformatted, 0 files already formatted\n"
        };
        assert_eq!(stderr(&output), summary, "{action:?}");
    }
    let output = run(&[&["-q"][..], &args].concat(), None, Some(&*dir));
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stderr(&output), "");
    let output = run(&[&["-v"][..], &args].concat(), None, Some(&*dir));
    assert_eq!(output.status.code(), Some(0));
    let errors = stderr(&output).replace('\\', "/");
    let lines: Vec<&str> = errors.lines().collect();
    assert!(
        lines[1].starts_with("vendor/b.sql: skipped, excluded by `vendor/` in /")
            && lines[1].ends_with("/.chlignore"),
        "{errors}"
    );
    assert_eq!(
        lines[2],
        "c.gen.sql: skipped, excluded by --exclude `*.gen.sql`"
    );
    assert_eq!(read(&dir.join("vendor/b.sql")), MESSY);
}

/// When `--stdin-filename` names an excluded file, which need not exist,
/// the text comes back as it is, even text that could not be formatted.
#[test]
fn stdin_for_an_excluded_file_comes_back_unchanged() {
    let dir = temp_dir("stdin-excluded");
    write(&dir, ".chlignore", "vendor/\n");
    fs::create_dir(dir.join("sub")).unwrap();
    let text = b"select 'unclosed \xff  \n\n\n";
    let cases: [(&[&str], &Path); 4] = [
        (&["--stdin-filename", "vendor/new/x.sql"], &dir),
        (&["--stdin-filename", "../vendor/x.sql"], &dir.join("sub")),
        (
            &[
                "--exclude",
                "*.gen.sql",
                "--stdin-filename",
                "sub/q.gen.sql",
            ],
            &dir,
        ),
        (
            &["--exclude", "/q.sql", "--stdin-filename", "q.sql"],
            &dir.join("sub"),
        ),
    ];
    for (args, cwd) in cases {
        let output = run(&[args, &["-"]].concat(), Some(text), Some(cwd));
        assert_eq!(
            output.status.code(),
            Some(0),
            "{args:?}: {}",
            stderr(&output)
        );
        assert_eq!(output.stdout, text, "{args:?}");
        assert_eq!(stderr(&output), "", "{args:?}");
        for action in ["--check", "--diff"] {
            let output = run(&[args, &[action, "-"]].concat(), Some(text), Some(cwd));
            assert_eq!(output.status.code(), Some(0), "{args:?} {action}");
            assert_eq!(stdout(&output), "", "{args:?} {action}");
            assert_eq!(stderr(&output), "", "{args:?} {action}");
        }
    }
    let output = run(
        &["-v", "--stdin-filename", "vendor/x.sql", "-"],
        Some(text),
        Some(&*dir),
    );
    assert_eq!(output.stdout, text);
    let errors = stderr(&output).replace('\\', "/");
    let skipped = errors.lines().nth(1).unwrap();
    assert!(
        skipped.starts_with("vendor/x.sql: skipped, excluded by `vendor/` in /")
            && skipped.ends_with("/.chlignore"),
        "{errors}"
    );
    // `/q.sql` is relative to the current directory.
    let output = run(
        &["--exclude", "/q.sql", "--stdin-filename", "sub/q.sql", "-"],
        Some(MESSY.as_bytes()),
        Some(&*dir),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), CLEAN);
}

/// The `chl.toml` of `dir`, as `chl` names it when run in `dir` or below:
/// with symbolic links resolved, as the current directory is.
fn config_file(dir: &Path) -> String {
    fs::canonicalize(dir)
        .unwrap()
        .join("chl.toml")
        .display()
        .to_string()
}

/// For stdin without --stdin-filename, the nearest `chl.toml`, in the
/// current directory or one above it, gives the settings; one further up
/// is not read.
#[test]
fn chl_toml_is_found_in_the_current_directory_or_above() {
    let dir = temp_dir("config-discovery");
    write(&dir, "chl.toml", "line-width = 20\njinja = \"on\"\n");
    let deep = dir.join("a/b");
    fs::create_dir_all(&deep).unwrap();
    let output = run(&["-"], Some(QUERY), Some(&deep));
    assert_eq!(stdout(&output), NARROW, "{}", stderr(&output));
    let output = run(&["-"], Some(TEMPLATE), Some(&deep));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    write(&dir, "a/chl.toml", "# Nothing set.\n");
    let output = run(&["-v", "-"], Some(QUERY), Some(&deep));
    assert_eq!(stdout(&output), WIDE);
    assert_eq!(
        stderr(&output),
        format!("settings from {}\n", config_file(&dir.join("a")))
    );
    let output = run(&["-"], Some(TEMPLATE), Some(&deep));
    assert_eq!(output.status.code(), Some(2));
}

/// For files, `chl.toml` is looked for from the deepest directory that
/// holds all the paths, wherever `chl` runs; one in a directory below that
/// is not read.
#[test]
fn chl_toml_is_found_from_the_paths() {
    let project = temp_dir("config-paths");
    let elsewhere = temp_dir("config-paths-elsewhere");
    write(&project, "chl.toml", "line-width = 20\n");
    write(&project, "x/chl.toml", "# Nothing set.\n");
    write(&project, "y/chl.toml", "# Nothing set.\n");
    let x = write(&project, "x/a.sql", WIDE);
    let y = write(&project, "y/b.sql", WIDE);
    let x = x.to_str().unwrap();
    let y = y.to_str().unwrap();
    let relative = format!(
        "../{}/x/a.sql",
        project.file_name().unwrap().to_str().unwrap()
    );
    for args in [&["-v", "--check", x][..], &["-v", "--check", &relative]] {
        let output = run(args, None, Some(&elsewhere));
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert!(
            stderr(&output).starts_with(&format!(
                "settings from {}\n",
                config_file(&project.join("x"))
            )),
            "{args:?}: {}",
            stderr(&output)
        );
    }
    let output = run(&["-v", "--check", x, y], None, Some(&elsewhere));
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).starts_with(&format!("settings from {}\n", config_file(&project))),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        stdout(&output),
        format!("Would reformat: {x}\nWould reformat: {y}\n")
    );
}

/// pre-commit and CI run `chl` at the root of the repository, with paths
/// relative to it: the `chl.toml` at the root gives the settings, as it
/// does for a run on the whole tree.
#[test]
fn chl_toml_at_the_root_serves_paths_relative_to_it() {
    let dir = temp_dir("config-root-relative");
    write(&dir, "chl.toml", "line-width = 20\n");
    for name in ["c.sql", "models/a.sql", "models/sub/b.sql"] {
        write(&dir, name, WIDE);
    }
    let runs: [&[&str]; 4] = [
        &["models/a.sql"],
        &["models/a.sql", "models/sub/b.sql"],
        &["models/sub/b.sql", "c.sql"],
        &["."],
    ];
    for paths in runs {
        let args = [&["-v", "--check"][..], paths].concat();
        let output = run(&args, None, Some(&dir));
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(
            stderr(&output).starts_with(&format!("settings from {}\n", config_file(&dir))),
            "{args:?}: {}",
            stderr(&output)
        );
    }
}

/// An editor runs `chl -` wherever it likes and names the buffer's file
/// with --stdin-filename: `chl.toml` is looked for from that file's
/// directory, or from the nearest one above it that exists.
#[test]
fn chl_toml_is_found_from_the_stdin_filename() {
    let project = temp_dir("config-stdin-filename");
    let elsewhere = temp_dir("config-stdin-filename-elsewhere");
    write(&project, "chl.toml", "line-width = 20\n");
    write(&project, "models/a.sql", "");
    let relative = format!(
        "../{}/models/q.sql",
        project.file_name().unwrap().to_str().unwrap()
    );
    for filename in [
        project.join("models/q.sql").display().to_string(),
        relative,
        project.join("no/such/dir/q.sql").display().to_string(),
        project.join("models/a.sql/q.sql").display().to_string(),
    ] {
        let output = run(
            &["-v", "--stdin-filename", &filename, "-"],
            Some(QUERY),
            Some(&elsewhere),
        );
        assert_eq!(stdout(&output), NARROW, "{filename}: {}", stderr(&output));
        assert_eq!(
            stderr(&output),
            format!("settings from {}\n", config_file(&project)),
            "{filename}"
        );
    }
}

/// A flag given on the command line wins over `chl.toml`, even when it
/// gives the default.
#[test]
fn a_flag_overrides_chl_toml() {
    let dir = temp_dir("config-precedence");
    write(&dir, "chl.toml", "line-width = 20\njinja = \"on\"\n");
    let output = run(&["-"], Some(QUERY), Some(&dir));
    assert_eq!(stdout(&output), NARROW);
    let output = run(&["--line-width", "100", "-"], Some(QUERY), Some(&dir));
    assert_eq!(stdout(&output), WIDE);
    let output = run(&["--line-width=30", "-"], Some(QUERY), Some(&dir));
    assert_eq!(stdout(&output), WIDE);
    let output = run(&["--jinja", "auto", "-"], Some(TEMPLATE), Some(&dir));
    assert_eq!(output.status.code(), Some(2));
    write(&dir, "chl.toml", "jinja = \"off\"\n");
    write(&dir, "t.sql.j2", std::str::from_utf8(TEMPLATE).unwrap());
    let output = run(&["--check", "t.sql.j2"], None, Some(&dir));
    assert_eq!(output.status.code(), Some(2));
    let output = run(
        &["--check", "--jinja", "auto", "t.sql.j2"],
        None,
        Some(&dir),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

/// `exclude` globs are relative to the directory of `chl.toml`, wherever
/// `chl` runs; `--exclude` adds to them, and cannot bring back what they
/// exclude.
#[test]
fn chl_toml_excludes_are_relative_to_its_directory() {
    let dir = temp_dir("config-exclude");
    write(
        &dir,
        "chl.toml",
        "exclude = [\"/models/vendor/\", \"*.gen.sql\", \"/top.sql\"]\n",
    );
    for name in [
        "top.sql",
        "models/top.sql",
        "models/a.sql",
        "models/b.gen.sql",
        "models/vendor/v.sql",
        "models/sub/c.sql",
    ] {
        write(&dir, name, "SELECT 1; ");
    }
    let models = dir.join("models");
    assert_eq!(
        listed(&models, &["."]),
        ["./a.sql", "./sub/c.sql", "./top.sql"]
    );
    assert_eq!(
        listed(&dir, &["."]),
        ["./models/a.sql", "./models/sub/c.sql", "./models/top.sql"]
    );
    assert_eq!(
        listed(
            &models,
            &[
                "a.sql",
                "b.gen.sql",
                "vendor/v.sql",
                "top.sql",
                "../top.sql"
            ]
        ),
        ["a.sql", "top.sql"]
    );
    assert_eq!(
        listed(
            &models,
            &["--exclude", "/sub", "--exclude", "!*.gen.sql", "."]
        ),
        ["./a.sql", "./top.sql"]
    );
    let output = run(&["--check", "vendor"], None, Some(&models));
    assert_eq!(output.status.code(), Some(0));
    assert!(
        stderr(&output).contains(&format!(
            "⚠ vendor: not searched, excluded by `/models/vendor/` in {}\n",
            config_file(&dir)
        )),
        "{}",
        stderr(&output)
    );
}

/// A path that several excludes match is reported as excluded by
/// `.chlignore` first, then by `chl.toml`, then by `--exclude`.
#[test]
fn chl_toml_excludes_named_files_and_stdin() {
    let dir = temp_dir("config-exclude-named");
    write(&dir, "chl.toml", "exclude = [\"vendor/\", \"*.gen.sql\"]\n");
    write(&dir, ".chlignore", "vendor/\n");
    for name in ["vendor/a.sql", "b.gen.sql", "c.sql"] {
        write(&dir, name, MESSY);
    }
    let output = run(
        &[
            "-v",
            "--exclude",
            "*.sql",
            "vendor/a.sql",
            "b.gen.sql",
            "c.sql",
        ],
        None,
        Some(&dir),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let errors = stderr(&output);
    let lines: Vec<&str> = errors.lines().collect();
    assert!(
        lines[1].starts_with("vendor/a.sql: skipped, excluded by `vendor/` in /")
            && lines[1].ends_with("/.chlignore"),
        "{errors}"
    );
    assert_eq!(
        lines[2],
        format!(
            "b.gen.sql: skipped, excluded by `*.gen.sql` in {}",
            config_file(&dir)
        )
    );
    assert_eq!(lines[3], "c.sql: skipped, excluded by --exclude `*.sql`");
    for name in ["vendor/a.sql", "b.gen.sql", "c.sql"] {
        assert_eq!(read(&dir.join(name)), MESSY, "{name}");
    }

    fs::create_dir(dir.join("sub")).unwrap();
    let text = b"select 'unclosed \xff  \n";
    for (filename, cwd) in [("sub/x.gen.sql", &*dir), ("x.gen.sql", &dir.join("sub"))] {
        for action in [None, Some("--check"), Some("--diff")] {
            let args = [
                &["--stdin-filename", filename][..],
                action.as_slice(),
                &["-"],
            ]
            .concat();
            let output = run(&args, Some(text), Some(cwd));
            assert_eq!(output.status.code(), Some(0), "{args:?}");
            let expected: &[u8] = if action.is_none() { text } else { b"" };
            assert_eq!(output.stdout, expected, "{args:?}");
            assert_eq!(stderr(&output), "", "{args:?}");
        }
    }
}

/// A `chl.toml` that cannot be used is an error before anything is
/// formatted, from stdin as from files, and a flag does not make up for it.
#[test]
fn an_invalid_chl_toml_exits_with_2() {
    let dir = temp_dir("config-errors");
    write(&dir, "sub/q.sql", CLEAN);
    let cases = [
        (
            "line-width = 0\n",
            ":1:14: invalid value: integer `0`, expected a nonzero u16",
        ),
        (
            "line-width = 65536\n",
            ":1:14: invalid value: integer `65536`, expected a nonzero u16",
        ),
        (
            "line-width = \"wide\"\n",
            ":1:14: invalid type: string \"wide\", expected a nonzero u16",
        ),
        (
            "jinja = \"maybe\"\n",
            ":1:9: unknown variant `maybe`, expected one of `auto`, `on`, `off`",
        ),
        (
            "exclude = \"vendor/\"\n",
            ":1:11: invalid type: string \"vendor/\", expected a sequence",
        ),
        (
            "line_width = 80\n",
            ":1:1: unknown field `line_width`, expected one of `line-width`, `jinja`, `exclude`",
        ),
        ("line-width = 80\nline-width = 90\n", ":2:1: duplicate key"),
        ("[jinja\n", ":1:7: unclosed table, expected `]`"),
        (
            "exclude = [\"a[b\"]\n",
            ": invalid exclude: error parsing glob 'a[b'",
        ),
    ];
    let config = config_file(&dir);
    let sub = dir.join("sub");
    for (toml, message) in cases {
        write(&dir, "chl.toml", toml);
        let runs: [(&[&str], Option<&[u8]>); 3] = [
            (&["--check", "q.sql"], None),
            (
                &["--line-width", "80", "--jinja", "off", "-"],
                Some(b"SELECT 1;\n"),
            ),
            (
                &["-q", "--stdin-filename", "q.sql", "-"],
                Some(b"SELECT 1;\n"),
            ),
        ];
        for (args, stdin) in runs {
            let output = run(args, stdin, Some(&sub));
            assert_eq!(output.status.code(), Some(2), "{toml:?} {args:?}");
            assert_eq!(stdout(&output), "", "{toml:?} {args:?}");
            assert!(
                stderr(&output).contains(&format!("{config}{message}")),
                "{toml:?} {args:?}: {}",
                stderr(&output)
            );
        }
    }
    assert_eq!(read(&sub.join("q.sql")), CLEAN);
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
        let mut command = chl(&dir);
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
    let output = run(&["-"], Some(QUERY), None);
    assert_eq!(stdout(&output), WIDE);
    let output = run(&["--line-width", "30", "-"], Some(QUERY), None);
    assert_eq!(stdout(&output), WIDE);
    let output = run(&["--line-width", "20", "-"], Some(QUERY), None);
    assert_eq!(stdout(&output), NARROW);
    let dir = temp_dir("line-width");
    write(&dir, "q.sql", WIDE);
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
    let config = dir
        .join("chl.toml")
        .display()
        .to_string()
        .replace('\\', "/");
    assert_eq!(
        lines[..3],
        [
            &format!("settings from {config}"),
            "./clean.sql: already formatted",
            "./messy.sql: would be reformatted"
        ]
    );
    assert!(
        lines[3].starts_with("1 file would be reformatted, 1 file already formatted in "),
        "{errors}"
    );
    assert!(lines[3].ends_with(" s"), "{errors}");
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
    let dir = temp_dir("colour");
    let has_escape = |output: &Output| stderr(output).contains('\u{1b}');
    let output = chl(&dir)
        .args(["--color", "always", "nope.sql"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(has_escape(&output), "{}", stderr(&output));
    let output = chl(&dir)
        .args(["--color", "auto", "nope.sql"])
        .env_remove("NO_COLOR")
        .output()
        .unwrap();
    assert!(
        !has_escape(&output),
        "stderr is a pipe: {}",
        stderr(&output)
    );
    let output = chl(&dir)
        .args(["--color", "never", "nope.sql"])
        .output()
        .unwrap();
    assert!(!has_escape(&output), "{}", stderr(&output));
}
