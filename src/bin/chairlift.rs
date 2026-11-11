//! `chairlift` — command-line formatter for ClickHouse SQL.
//!
//! Each file is handled on its own by [`process`], which reads, formats and
//! (unless checking) writes it and returns an [`Outcome`] or a [`FileError`];
//! reporting and the exit code are worked out from those afterwards.

#[path = "chairlift/diagnostic.rs"]
mod diagnostic;
#[path = "chairlift/parallel.rs"]
mod parallel;

use std::fs;
use std::io;
use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::LazyLock;
use std::time::Instant;

use chairlift::Options;
use chairlift::lexer::Mode;
use clap::Parser;
use clap::ValueEnum;
use diagnostic::Color;
use diagnostic::Diagnostic;
use diagnostic::Reporter;
use diagnostic::Verbosity;

/// Format ClickHouse SQL and Jinja-templated SQL files.
///
/// Files are formatted in place. Directories are searched for `*.sql`,
/// `*.sql.j2` and `*.j2.sql` files, skipping hidden files and whatever
/// `.gitignore` and `.ignore` files exclude.
#[derive(Debug, Parser)]
#[allow(clippy::struct_excessive_bools, reason = "each is a flag")]
#[command(
    version = VERSION.as_str(),
    after_help = "Exit status: 0 when done, 1 when --check or --diff found files that would \
                  change, 2 when a file could not be formatted (unreadable, not UTF-8, a lexer \
                  error, a failed verification, or a template refused because its rendering \
                  decides where a line comment ends) or the arguments are wrong."
)]
struct Cli {
    /// Files or directories to format; `-` reads stdin and writes stdout.
    #[arg(required = true)]
    paths: Vec<PathBuf>,

    /// Write nothing; list the files that would change and exit with 1 if
    /// there are any.
    #[arg(long)]
    check: bool,

    /// Write nothing; print a unified diff of the changes and exit with 1 if
    /// there are any.
    #[arg(long)]
    diff: bool,

    /// The width to lay lines out to.
    #[arg(long, value_name = "WIDTH", default_value_t = Options::DEFAULT_LINE_WIDTH,
          value_parser = clap::value_parser!(u16).range(1..))]
    line_width: u16,

    /// Whether files are Jinja templates; `auto` decides by name (`*.sql.j2`
    /// and `*.j2.sql` are), and stdin is not.
    #[arg(long, value_enum, value_name = "WHEN", default_value_t = Jinja::Auto)]
    jinja: Jinja,

    /// Files formatted at once [default: the number of CPUs]. Results are
    /// reported in path order whatever the number.
    #[arg(short, long, value_name = "N", value_parser = clap::value_parser!(u16).range(1..))]
    jobs: Option<u16>,

    /// Print only errors, besides the formatted text, the list of files or
    /// the diff that is asked for: no summary, warnings or notes.
    #[arg(short, long, conflicts_with = "verbose")]
    quiet: bool,

    /// Also print what happened to each file, and how long it all took.
    #[arg(short, long)]
    verbose: bool,

    /// When to colour diagnostics; `auto` colours them when stderr is a
    /// terminal and `NO_COLOR` is not set.
    #[arg(long, value_enum, value_name = "WHEN", default_value_t = Color::Auto)]
    color: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Jinja {
    Auto,
    On,
    Off,
}

/// `<version> (ClickHouse <version>)`, so a bug report names both.
static VERSION: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{} (ClickHouse {})",
        env!("CARGO_PKG_VERSION"),
        chairlift::CLICKHOUSE_VERSION
    )
});

/// What to do with a formatted file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Write,
    Check,
    Diff,
}

/// How a file is to be formatted and what to do with the result.
#[derive(Clone, Debug)]
struct Settings {
    action: Action,
    line_width: u16,
    jinja: Jinja,
}

impl Settings {
    fn options(&self, path: Option<&Path>) -> Options {
        let jinja = match self.jinja {
            Jinja::On => true,
            Jinja::Off => false,
            Jinja::Auto => path.is_some_and(|p| Mode::for_path(p) == Mode::Jinja),
        };
        Options::default()
            .with_line_width(self.line_width)
            .with_jinja(jinja)
    }
}

/// A file's text and its formatted text.
#[derive(Debug)]
struct Formatted {
    before: String,
    after: String,
}

/// A file that was formatted.
#[derive(Debug)]
struct Outcome {
    /// Whether formatting changed it.
    changed: bool,
    /// The unified diff, when that is the action and the file changed.
    diff: Option<String>,
}

/// Why a file was left as it was.
#[derive(Debug)]
enum FileError {
    Read(io::Error),
    /// `text` is the file read lossily, for the snippet.
    NotUtf8 {
        valid_up_to: usize,
        text: String,
    },
    Format {
        error: chairlift::Error,
        text: String,
    },
    Write(io::Error),
}

impl FileError {
    /// Whether this is a template that chairlift leaves as it is because the
    /// rendering decides where one of its line comments ends: the file is
    /// fine, but cannot be formatted safely.
    fn is_refused_template(&self) -> bool {
        matches!(
            self,
            FileError::Format {
                error: chairlift::Error::Template(_),
                ..
            }
        )
    }

    /// The diagnostic for the file called `name`.
    fn diagnostic(self, name: &str) -> Diagnostic {
        match self {
            FileError::Read(e) => {
                Diagnostic::error("chairlift::read", format!("{name}: cannot be read: {e}"))
            }
            FileError::NotUtf8 { valid_up_to, text } => {
                let at = chairlift::Position::new(&text, valid_up_to);
                Diagnostic::error(
                    "chairlift::not_utf8",
                    format!(
                        "{name}:{at}: not UTF-8 (at byte {valid_up_to}); the file is left \
                         unchanged"
                    ),
                )
                .with_snippet(name, text, valid_up_to, "not UTF-8")
            }
            FileError::Format {
                error: chairlift::Error::Lex(e),
                text,
            } => Diagnostic::error(
                "chairlift::lex",
                format!("{name}:{e}; the file is left unchanged"),
            )
            .with_snippet(name, text, e.position.offset, "here"),
            FileError::Format {
                error: chairlift::Error::Template(e),
                text,
            } => Diagnostic::error(
                "chairlift::template",
                format!(
                    "{name}:{}: where this line comment ends depends on the rendering, so the \
                     file is left unchanged",
                    e.position
                ),
            )
            .with_snippet(name, text, e.position.offset, "this comment")
            .with_help(e.reason.to_string()),
            FileError::Format {
                error: chairlift::Error::Verify(e),
                text,
            } => Diagnostic::error(
                "chairlift::verify",
                format!(
                    "{name}: chairlift's output failed verification, so the file is left \
                     unchanged: {e}"
                ),
            )
            .with_snippet(name, text, e.input.offset, "the output differs here")
            .with_help(diagnostic::bug_report_help()),
            FileError::Format {
                error: chairlift::Error::Layout(e),
                text,
            } => Diagnostic::error(
                "chairlift::layout",
                format!(
                    "{name}: chairlift's layout lost or repeated a token of it, so the file is \
                     left unchanged: {}",
                    e.detail
                ),
            )
            .with_snippet(name, text, e.position.offset, "this token")
            .with_help(diagnostic::bug_report_help()),
            FileError::Format { error, .. } => Diagnostic::error(
                "chairlift::format",
                format!("{name}: {error}; the file is left unchanged"),
            ),
            FileError::Write(e) => {
                let diagnostic = Diagnostic::error(
                    "chairlift::write",
                    format!("{name}: cannot be written: {e}; the file is left as it was"),
                );
                if e.kind() == io::ErrorKind::PermissionDenied {
                    diagnostic.with_help(
                        "make the file writable, or use --check or --diff, which only read it",
                    )
                } else {
                    diagnostic
                }
            }
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let reporter = Reporter {
        verbosity: if cli.quiet {
            Verbosity::Quiet
        } else if cli.verbose {
            Verbosity::Verbose
        } else {
            Verbosity::Normal
        },
        color: cli.color.for_stderr(),
    };
    let settings = Settings {
        action: if cli.diff {
            Action::Diff
        } else if cli.check {
            Action::Check
        } else {
            Action::Write
        },
        line_width: cli.line_width,
        jinja: cli.jinja,
    };
    if cli.paths.iter().any(|p| p.as_os_str() == "-") {
        if cli.paths.len() > 1 {
            eprintln!("error: `-` (stdin) cannot be combined with other paths");
            return ExitCode::from(2);
        }
        return stdin(&settings, reporter);
    }
    let jobs = cli.jobs.map_or_else(parallel::default_jobs, usize::from);
    files(&cli.paths, &settings, jobs, reporter)
}

/// Formats stdin to stdout, or checks or diffs it.
fn stdin(settings: &Settings, reporter: Reporter) -> ExitCode {
    let result = read_stdin()
        .and_then(|source| format_text(source, &settings.options(None)))
        .and_then(|formatted| {
            let outcome = finish("-", None, &formatted, settings)?;
            Ok((formatted, outcome))
        });
    let (formatted, outcome) = match result {
        Ok(done) => done,
        Err(e) => {
            report(reporter, &e.diagnostic("<stdin>"));
            return ExitCode::from(2);
        }
    };
    let mut stdout = io::stdout().lock();
    let written = match (settings.action, outcome.diff) {
        (Action::Write, _) => stdout.write_all(formatted.after.as_bytes()),
        (Action::Check, _) if outcome.changed => writeln!(stdout, "Would reformat: -"),
        (_, Some(diff)) => stdout.write_all(diff.as_bytes()),
        (Action::Check | Action::Diff, None) => Ok(()),
    };
    if let Err(e) = written.and_then(|()| stdout.flush()) {
        eprintln!("error: cannot write to stdout: {e}");
        return ExitCode::from(2);
    }
    if settings.action != Action::Write && outcome.changed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn read_stdin() -> Result<String, FileError> {
    let mut bytes = Vec::new();
    io::stdin()
        .read_to_end(&mut bytes)
        .map_err(FileError::Read)?;
    String::from_utf8(bytes).map_err(not_utf8)
}

fn not_utf8(e: std::string::FromUtf8Error) -> FileError {
    let valid_up_to = e.utf8_error().valid_up_to();
    FileError::NotUtf8 {
        valid_up_to,
        text: String::from_utf8_lossy(&e.into_bytes()).into_owned(),
    }
}

/// Prints `diagnostic` to stderr, unless the verbosity leaves it out.
fn report(reporter: Reporter, diagnostic: &Diagnostic) {
    if let Some(text) = reporter.render(diagnostic) {
        eprint!("{text}");
    }
}

/// Formats, checks or diffs the files under `paths`.
fn files(paths: &[PathBuf], settings: &Settings, jobs: usize, reporter: Reporter) -> ExitCode {
    let started = Instant::now();
    let (files, mut errors) = discover(paths, reporter);
    let outcomes = parallel::map(&files, jobs, |path| process(path, settings));
    let mut changed = 0;
    let mut unchanged = 0;
    let mut refused = 0;
    let mut stdout = io::stdout().lock();
    for (path, outcome) in files.iter().zip(outcomes) {
        let shown = path.display().to_string();
        match outcome {
            Ok(Outcome { changed: false, .. }) => {
                unchanged += 1;
                if reporter.verbosity == Verbosity::Verbose {
                    eprintln!("{shown}: already formatted");
                }
            }
            Ok(Outcome {
                changed: true,
                diff,
            }) => {
                changed += 1;
                if reporter.verbosity == Verbosity::Verbose {
                    match settings.action {
                        Action::Write => eprintln!("{shown}: reformatted"),
                        Action::Check | Action::Diff => {
                            eprintln!("{shown}: would be reformatted");
                        }
                    }
                }
                let written = match (settings.action, diff) {
                    (Action::Check, _) => writeln!(stdout, "Would reformat: {shown}"),
                    (_, Some(diff)) => stdout.write_all(diff.as_bytes()),
                    (_, None) => Ok(()),
                };
                if let Err(e) = written {
                    eprintln!("error: cannot write to stdout: {e}");
                    return ExitCode::from(2);
                }
            }
            Err(e) => {
                if e.is_refused_template() {
                    refused += 1;
                } else {
                    errors += 1;
                }
                report(reporter, &e.diagnostic(&shown));
            }
        }
    }
    drop(stdout);
    let summary = summary(settings.action, changed, unchanged, refused, errors);
    match reporter.verbosity {
        Verbosity::Quiet => {}
        Verbosity::Normal => eprintln!("{summary}"),
        Verbosity::Verbose => eprintln!("{summary} in {:.2} s", started.elapsed().as_secs_f64()),
    }
    if errors > 0 || refused > 0 {
        ExitCode::from(2)
    } else if changed > 0 && settings.action != Action::Write {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// The files to format: each path that is a file, and the SQL files under
/// each path that is a directory, sorted and without duplicates. Paths that
/// cannot be read are reported, and counted in the second value.
fn discover(paths: &[PathBuf], reporter: Reporter) -> (Vec<PathBuf>, usize) {
    let mut files = Vec::new();
    let mut errors = 0;
    for path in paths {
        match fs::metadata(path) {
            Ok(meta) if meta.is_dir() => {
                let walk = ignore::WalkBuilder::new(path).require_git(false).build();
                for entry in walk {
                    match entry {
                        Ok(entry) => {
                            if entry.file_type().is_some_and(|t| t.is_file())
                                && is_sql_file(entry.path())
                            {
                                files.push(entry.into_path());
                            }
                        }
                        Err(e) => {
                            report(
                                reporter,
                                &Diagnostic::error("chairlift::walk", e.to_string()),
                            );
                            errors += 1;
                        }
                    }
                }
            }
            Ok(_) => files.push(path.clone()),
            Err(e) => {
                report(
                    reporter,
                    &FileError::Read(e).diagnostic(&path.display().to_string()),
                );
                errors += 1;
            }
        }
    }
    files.sort();
    files.dedup();
    (files, errors)
}

/// `*.sql` (which includes `*.j2.sql`) or `*.sql.j2`, in any case.
fn is_sql_file(path: &Path) -> bool {
    let has = |p: &Path, ext: &str| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext));
    has(path, "sql")
        || (has(path, "j2")
            && path
                .file_stem()
                .is_some_and(|stem| has(Path::new(stem), "sql")))
}

/// Formats one file, writing it back if that is the action and it changed.
fn process(path: &Path, settings: &Settings) -> Result<Outcome, FileError> {
    let formatted = format_file(path, settings)?;
    finish(
        &path.display().to_string(),
        Some(path),
        &formatted,
        settings,
    )
}

/// Reads and formats one file.
fn format_file(path: &Path, settings: &Settings) -> Result<Formatted, FileError> {
    let bytes = fs::read(path).map_err(FileError::Read)?;
    let before = String::from_utf8(bytes).map_err(not_utf8)?;
    format_text(before, &settings.options(Some(path)))
}

fn format_text(before: String, options: &Options) -> Result<Formatted, FileError> {
    match chairlift::format(&before, options) {
        Ok(after) => Ok(Formatted { before, after }),
        Err(error) => Err(FileError::Format {
            error,
            text: before,
        }),
    }
}

/// Does what the action asks with a formatted file called `name`: writes it
/// back to `path`, if it has one, or works out its diff.
fn finish(
    name: &str,
    path: Option<&Path>,
    formatted: &Formatted,
    settings: &Settings,
) -> Result<Outcome, FileError> {
    let Formatted { before, after } = formatted;
    let changed = before != after;
    let mut diff = None;
    if changed {
        match (settings.action, path) {
            (Action::Write, Some(path)) => {
                write_atomically(path, after).map_err(FileError::Write)?;
            }
            (Action::Diff, _) => diff = Some(unified_diff(name, before, after)),
            (Action::Write | Action::Check, _) => {}
        }
    }
    Ok(Outcome { changed, diff })
}

/// Replaces the file at `path` with `contents` so that a crash leaves either
/// the old file or the new one, never a truncated one: the text goes to a
/// temporary file in the same directory, is synced, and is renamed over the
/// original. A symbolic link is followed, so the file it points at is
/// replaced and the link stays. The new file gets the old one's permissions.
///
/// Renaming needs write permission on the directory only, so a file the
/// user may not write is refused first, as writing it in place would be.
fn write_atomically(path: &Path, contents: &str) -> io::Result<()> {
    let target = fs::canonicalize(path)?;
    fs::OpenOptions::new().write(true).open(&target)?;
    let dir = target
        .parent()
        .ok_or_else(|| io::Error::other("the file has no parent directory"))?;
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::other("the path has no file name"))?;
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(name);
    temp_name.push(format!(".chairlift-{}.tmp", std::process::id()));
    let temp = dir.join(temp_name);
    let permissions = fs::metadata(&target)?.permissions();
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(contents.as_bytes())?;
        file.set_permissions(permissions)?;
        file.sync_all()?;
        fs::rename(&temp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn unified_diff(name: &str, before: &str, after: &str) -> String {
    similar::TextDiff::from_lines(before, after)
        .unified_diff()
        .header(&format!("a/{name}"), &format!("b/{name}"))
        .to_string()
}

/// The line that sums up a run: how many files changed or would change,
/// were already formatted, were templates left as they are, and failed.
fn summary(
    action: Action,
    changed: usize,
    unchanged: usize,
    refused: usize,
    errors: usize,
) -> String {
    let files = |n: usize| {
        if n == 1 {
            "1 file".to_owned()
        } else {
            format!("{n} files")
        }
    };
    let mut parts = vec![match action {
        Action::Write => format!("{} reformatted", files(changed)),
        Action::Check | Action::Diff => format!("{} would be reformatted", files(changed)),
    }];
    parts.push(format!("{} already formatted", files(unchanged)));
    match refused {
        0 => {}
        1 => parts.push("1 template refused".to_owned()),
        n => parts.push(format!("{n} templates refused")),
    }
    match errors {
        0 => {}
        1 => parts.push("1 error".to_owned()),
        n => parts.push(format!("{n} errors")),
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(error: FileError, name: &str) -> String {
        let reporter = Reporter {
            verbosity: Verbosity::Normal,
            color: false,
        };
        reporter
            .render(&error.diagnostic(name))
            .expect("errors are always shown")
    }

    #[test]
    fn a_verification_failure_asks_for_a_bug_report() {
        let error = FileError::Format {
            error: chairlift::Error::Verify(chairlift::verify::VerifyError {
                input: chairlift::Position::new("a -1", 2),
                output: chairlift::Position::new("a--1", 1),
                detail: "token 2 differs".to_owned(),
            }),
            text: "a -1".to_owned(),
        };
        insta::assert_snapshot!(render(error, "q.sql"), @"
        chairlift::verify

          × q.sql: chairlift's output failed verification, so the file is left unchanged: the output differs from the input at input 1:3, output 1:2: token 2 differs
           ╭─[q.sql:1:3]
         1 │ a -1
           ·   ┬
           ·   ╰── the output differs here
           ╰────
          help: this is a bug in chairlift. Please report it at https://github.com/glennib/chairlift/issues with the file, or the smallest part of it that shows the problem, and the output of `chairlift --version`.
        ");
    }

    #[test]
    fn a_line_too_long_to_show_has_no_snippet() {
        let text = format!("SELECT '{}", "x".repeat(1000));
        let error = chairlift::format(&text, &Options::default()).unwrap_err();
        insta::assert_snapshot!(render(FileError::Format { error, text }, "long.sql"), @"
        chairlift::lex

          × long.sql:1:8: Single quoted string is not closed; the file is left unchanged
        ");
    }

    #[test]
    fn summaries() {
        assert_eq!(
            summary(Action::Write, 1, 0, 0, 0),
            "1 file reformatted, 0 files already formatted"
        );
        assert_eq!(
            summary(Action::Check, 2, 3, 0, 1),
            "2 files would be reformatted, 3 files already formatted, \
             1 error"
        );
        assert_eq!(
            summary(Action::Diff, 0, 1, 2, 0),
            "0 files would be reformatted, 1 file already formatted, \
             2 templates refused"
        );
    }

    /// A template the rendering makes unsafe to format is not a bug: the
    /// diagnostic points at the comment and says what to change.
    #[test]
    fn a_refused_template_names_its_comment_and_the_fix() {
        let text = "select a -- c\n{%- if x %}\n, b\n{% endif %} from t\n".to_owned();
        let error = chairlift::format(&text, &Options::default().with_jinja(true)).unwrap_err();
        let error = FileError::Format { error, text };
        assert!(error.is_refused_template());
        insta::assert_snapshot!(render(error, "m.sql.j2"), @"
        chairlift::template

          × m.sql.j2:1:10: where this line comment ends depends on the rendering, so the file is left unchanged
           ╭─[m.sql.j2:1:10]
         1 │ select a -- c
           ·          ┬
           ·          ╰── this comment
         2 │ {%- if x %}
           ╰────
          help: the `-` marker of the tag after it strips the line break after it, and the comment can run on into text that follows a tag on its line; remove the marker, or end the lines of the tags there
        ");
    }
}
