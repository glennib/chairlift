//! `chl`, chairlift's command line: a formatter for ClickHouse SQL.
//!
//! Each file is handled on its own: read and formatted ([`format_file`]),
//! judged by `clickhouse-format` with `--verify` ([`verify()`]), and written
//! back or diffed ([`finish`]), giving an [`Outcome`] or a [`FileError`];
//! files are handled in parallel, and reported in path order afterwards.

/// `eprintln!` that does not panic when stderr is closed or a broken pipe,
/// as when it is piped into `head`: what cannot be said is dropped, and the
/// run goes on to the exit status it would have had.
macro_rules! say {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

#[path = "chl/diagnostic.rs"]
mod diagnostic;
#[path = "chl/parallel.rs"]
mod parallel;
#[path = "chl/verify.rs"]
mod verify;

use std::fs;
use std::io;
use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::LazyLock;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Instant;

use chairlift::Mode;
use chairlift::Options;
use chairlift::oracle::Counts;
use chairlift::oracle::RejectionClass;
use clap::Parser;
use clap::ValueEnum;
use diagnostic::Color;
use diagnostic::Diagnostic;
use diagnostic::Reporter;
use diagnostic::Verbosity;
use verify::Verifier;

/// Format ClickHouse SQL and Jinja-templated SQL files.
///
/// Files are formatted in place. Directories are searched for `*.sql`,
/// `*.sql.j2` and `*.j2.sql` files, skipping hidden files, symbolic links
/// and whatever `.gitignore` and `.ignore` files exclude. A file named on
/// the command line is formatted whatever its name.
#[derive(Debug, Parser)]
#[allow(clippy::struct_excessive_bools, reason = "each is a flag")]
#[command(
    version = VERSION.as_str(),
    long_version = LONG_VERSION.as_str(),
    after_help = "Exit status: 0 when done, 1 when --check or --diff found files that would \
                  change, 2 when a file could not be formatted (unreadable, not UTF-8, a lexer \
                  error, a failed verification, or a template refused because its rendering \
                  decides where a line comment ends), --verify cannot run clickhouse-format, \
                  or the arguments are wrong."
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
    /// there are any. With --check too, the diff is printed instead of the
    /// list.
    #[arg(long)]
    diff: bool,

    /// Also check with ClickHouse's own formatter, `clickhouse-format`, that
    /// ClickHouse reads each changed file as before. A file it reads
    /// differently is an error and is not written, and so is a file of which
    /// it judges nothing because it crashes, times out or prints too much on
    /// it; where that happens to only some statements of a file, a warning
    /// that --quiet does not hide says so. Files that formatting
    /// leaves unchanged are not judged, as there is nothing to compare;
    /// neither are statements it cannot parse, nor template statements that
    /// hold Jinja.
    #[arg(long)]
    verify: bool,

    /// The `clickhouse-format` program for --verify, or the `clickhouse`
    /// binary [default: `clickhouse-format` on PATH].
    #[arg(long, value_name = "PATH", requires = "verify")]
    clickhouse_binary: Option<PathBuf>,

    /// How long one run of `clickhouse-format` may take, in seconds; for
    /// tests.
    #[arg(long, value_name = "SECONDS", requires = "verify", hide = true)]
    verify_timeout: Option<f64>,

    /// The width to lay lines out to.
    #[arg(long, value_name = "WIDTH", default_value_t = Options::DEFAULT_LINE_WIDTH,
          value_parser = clap::value_parser!(u16).range(1..))]
    line_width: u16,

    /// Whether files are Jinja templates; `auto` decides by name (`*.j2`,
    /// such as `*.sql.j2`, and `*.j2.sql` are), and stdin is not unless
    /// --stdin-filename names a template.
    #[arg(long, value_enum, value_name = "WHEN", default_value_t = Jinja::Auto)]
    jinja: Jinja,

    /// The name of the file whose text `-` reads from stdin, as an editor
    /// passes it: `--jinja auto` decides by it, and messages and diffs show
    /// it. The file itself is not read.
    #[arg(long, value_name = "PATH")]
    stdin_filename: Option<PathBuf>,

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

/// `<version> (ClickHouse keywords <version>)`, so a bug report names both.
static VERSION: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{} (ClickHouse keywords {})",
        env!("CARGO_PKG_VERSION"),
        chairlift::KEYWORDS_VERSION
    )
});

/// [`VERSION`], then the optional features the binary was built with, for
/// `--version`.
static LONG_VERSION: LazyLock<String> = LazyLock::new(|| {
    let features: Vec<&str> = [
        ("oracle", cfg!(feature = "oracle")),
        ("template", cfg!(feature = "template")),
    ]
    .into_iter()
    .filter_map(|(name, on)| on.then_some(name))
    .collect();
    let features = if features.is_empty() {
        "none".to_owned()
    } else {
        features.join(", ")
    };
    format!("{}\nfeatures: {features}", *VERSION)
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

/// A file read and formatted, for `--verify` to judge.
struct Item {
    /// The file's name in messages.
    name: String,
    mode: Mode,
    result: Result<Formatted, FileError>,
}

/// What `--verify` found.
struct Verification {
    /// Statement counts over every changed file.
    counts: Counts,
    /// Templates judged without rendering.
    unrendered: usize,
    /// Notes and warnings per item.
    notes: Vec<Vec<Diagnostic>>,
    /// The error saying the oracle stopped running, if it did.
    gave_up: Option<Diagnostic>,
}

/// Judges the changed files among `items` with the oracle, turning each one
/// it reads differently into an error.
fn verify(verifier: &Verifier, items: &mut [Item]) -> Verification {
    let changed: Vec<usize> = (0..items.len())
        .filter(|&i| matches!(&items[i].result, Ok(f) if f.before != f.after))
        .collect();
    let jobs: Vec<verify::Job<'_>> = changed
        .iter()
        .filter_map(|&i| {
            let item = &items[i];
            let formatted = item.result.as_ref().ok()?;
            Some(verify::Job {
                name: &item.name,
                input: &formatted.before,
                output: &formatted.after,
                mode: item.mode,
            })
        })
        .collect();
    let verdicts = verifier.verify(&jobs);
    drop(jobs);
    let mut verification = Verification {
        counts: Counts::default(),
        unrendered: 0,
        notes: items.iter().map(|_| Vec::new()).collect(),
        gave_up: verifier.gave_up(),
    };
    for (i, verdict) in changed.into_iter().zip(verdicts) {
        verification.counts.add(&verdict.counts);
        let name = &items[i].name;
        let notes = &mut verification.notes[i];
        if verdict.unrendered {
            verification.unrendered += 1;
            let jinja = verdict.counts.rejected(RejectionClass::UnrenderedJinja);
            notes.push(Diagnostic::note(
                "chairlift::verify::template",
                format!(
                    "{name}: a template, judged without rendering it: {} with Jinja not judged",
                    statements(jinja)
                ),
            ));
        }
        let failed = verdict.counts.rejected(RejectionClass::Failed);
        // A file the oracle judged nothing of has not been verified at all,
        // which `--verify` must not pass in silence; once the oracle gave
        // up, what it failed on was not even tried.
        let unjudged =
            failed > 0 && (verdict.counts.judged() == 0 || verification.gave_up.is_some());
        if failed > 0 && !unjudged {
            notes.push(
                Diagnostic::warning(
                    "chairlift::verify::failed",
                    format!(
                        "{name}: clickhouse-format failed (crashed, timed out or printed too \
                         much) on {}, not judged",
                        statements(failed)
                    ),
                )
                .even_when_quiet(),
            );
        }
        notes.extend(verdict.whole_only);
        if let Some(bug) = verdict.bug {
            items[i].result = Err(FileError::Oracle(Box::new(bug)));
        } else if unjudged {
            let message = if verification.gave_up.is_some() {
                format!(
                    "{name}: {} not judged, as --verify stopped running clickhouse-format, so the \
                     file is left unchanged",
                    statements(failed)
                )
            } else {
                format!(
                    "{name}: clickhouse-format judged none of its statements, as it failed \
                     (crashed, timed out or printed too much) on {failed}, so the file is left \
                     unchanged"
                )
            };
            items[i].result = Err(FileError::Oracle(Box::new(Diagnostic::error(
                "chairlift::verify::failed",
                message,
            ))));
        }
    }
    verification
}

/// `n` statements, in words.
fn statements(n: usize) -> String {
    if n == 1 {
        "1 statement".to_owned()
    } else {
        format!("{n} statements")
    }
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
    /// `directory` when it was the file's directory that refused.
    Write {
        error: io::Error,
        directory: bool,
    },
    /// The file changed, or went away, after it was read.
    Changed,
    /// `--verify` found that ClickHouse reads the output differently.
    Oracle(Box<Diagnostic>),
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
            FileError::Oracle(diagnostic) => *diagnostic,
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
            FileError::Changed => Diagnostic::error(
                "chairlift::changed",
                format!(
                    "{name}: changed while chairlift was running, so it is left as it is; run \
                     chairlift again"
                ),
            ),
            FileError::Write { error, directory } => {
                let diagnostic = Diagnostic::error(
                    "chairlift::write",
                    format!("{name}: cannot be written: {error}; the file is left as it was"),
                );
                match (error.kind(), directory) {
                    (io::ErrorKind::PermissionDenied, false) => diagnostic.with_help(
                        "make the file writable, or use --check or --diff, which only read it",
                    ),
                    (io::ErrorKind::PermissionDenied, true) => diagnostic.with_help(
                        "the file's directory is not writable, and chairlift replaces a file by \
                         writing a new one beside it; make the directory writable, or use \
                         --check or --diff, which only read the file",
                    ),
                    _ => diagnostic,
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
            say!("error: `-` (stdin) cannot be combined with other paths");
            return ExitCode::from(2);
        }
    } else if cli.stdin_filename.is_some() {
        say!("error: --stdin-filename names the file on stdin, so it needs `-` as the path");
        return ExitCode::from(2);
    }
    let jobs = cli.jobs.map_or_else(parallel::default_jobs, usize::from);
    let verifier = if cli.verify {
        let timeout = cli
            .verify_timeout
            .and_then(|s| std::time::Duration::try_from_secs_f64(s).ok());
        match Verifier::new(cli.clickhouse_binary, jobs, timeout) {
            Ok(verifier) => Some(verifier),
            Err(diagnostic) => {
                report(reporter, &diagnostic);
                return ExitCode::from(2);
            }
        }
    } else {
        None
    };
    if cli.paths.iter().any(|p| p.as_os_str() == "-") {
        return stdin(
            &settings,
            cli.stdin_filename.as_deref(),
            verifier.as_ref(),
            reporter,
        );
    }
    files(&cli.paths, &settings, jobs, verifier.as_ref(), reporter)
}

/// Formats stdin to stdout, or checks or diffs it. `filename` is the name
/// of the file the text comes from, if `--stdin-filename` gives one.
fn stdin(
    settings: &Settings,
    filename: Option<&Path>,
    verifier: Option<&Verifier>,
    reporter: Reporter,
) -> ExitCode {
    let shown = filename.map(|f| f.display().to_string());
    let options = settings.options(filename);
    let mut items = vec![Item {
        name: shown.clone().unwrap_or_else(|| "<stdin>".to_owned()),
        mode: options.mode(),
        result: read_stdin().and_then(|source| format_text(source, &options)),
    }];
    if let Some(verifier) = verifier {
        let verification = verify(verifier, &mut items);
        for note in verification.notes.iter().flatten() {
            report(reporter, note);
        }
        if let Some(gave_up) = &verification.gave_up {
            report(reporter, gave_up);
        }
        if reporter.verbosity == Verbosity::Verbose {
            say!(
                "{}",
                verify::summary(&verification.counts, verification.unrendered)
            );
        }
    }
    let Some(item) = items.pop() else {
        return ExitCode::from(2);
    };
    let result = item.result.and_then(|formatted| {
        let outcome = finish(shown.as_deref().unwrap_or("-"), None, &formatted, settings)?;
        Ok((formatted, outcome))
    });
    let (formatted, outcome) = match result {
        Ok(done) => done,
        Err(e) => {
            report(reporter, &e.diagnostic(&item.name));
            return ExitCode::from(2);
        }
    };
    let mut stdout = io::stdout().lock();
    let written = match (settings.action, outcome.diff) {
        (Action::Write, _) => stdout.write_all(formatted.after.as_bytes()),
        (Action::Check, _) if outcome.changed => writeln!(
            stdout,
            "Would reformat: {}",
            shown.as_deref().unwrap_or("-")
        ),
        (_, Some(diff)) => stdout.write_all(diff.as_bytes()),
        (Action::Check | Action::Diff, None) => Ok(()),
    };
    match written.and_then(|()| stdout.flush()) {
        // Whoever reads stdout stopped reading: as for `head`, that is
        // theirs to decide.
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => {
            say!("error: cannot write to stdout: {e}");
            return ExitCode::from(2);
        }
        _ => {}
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
        let _ = io::stderr().write_all(text.as_bytes());
    }
}

/// Formats, checks or diffs the files under `paths`.
fn files(
    paths: &[PathBuf],
    settings: &Settings,
    jobs: usize,
    verifier: Option<&Verifier>,
    reporter: Reporter,
) -> ExitCode {
    let started = Instant::now();
    let (files, mut errors) = discover(paths, reporter);
    let (outcomes, verification) = match verifier {
        None => (
            parallel::map(files.iter().collect(), jobs, |path: &PathBuf| {
                process(path, settings)
            }),
            None,
        ),
        // Every file is formatted before any is written, so that the oracle
        // judges them all in one batched run.
        Some(verifier) => {
            let mut items = parallel::map(files.iter().collect(), jobs, |path: &PathBuf| Item {
                name: path.display().to_string(),
                mode: settings.options(Some(path)).mode(),
                result: format_file(path, settings),
            });
            let verification = verify(verifier, &mut items);
            let outcomes = parallel::map(
                files.iter().zip(items).collect(),
                jobs,
                |(path, item): (&PathBuf, Item)| {
                    item.result
                        .and_then(|formatted| finish(&item.name, Some(path), &formatted, settings))
                },
            );
            (outcomes, Some(verification))
        }
    };
    let mut changed = 0;
    let mut unchanged = 0;
    let mut refused = 0;
    let mut stdout = io::stdout().lock();
    let mut stdout_closed = false;
    for (i, (path, outcome)) in files.iter().zip(outcomes).enumerate() {
        let shown = path.display().to_string();
        let notes = verification.as_ref().and_then(|v| v.notes.get(i));
        for note in notes.into_iter().flatten() {
            report(reporter, note);
        }
        match outcome {
            Ok(Outcome { changed: false, .. }) => {
                unchanged += 1;
                if reporter.verbosity == Verbosity::Verbose {
                    say!("{shown}: already formatted");
                }
            }
            Ok(Outcome {
                changed: true,
                diff,
            }) => {
                changed += 1;
                if reporter.verbosity == Verbosity::Verbose {
                    match settings.action {
                        Action::Write => say!("{shown}: reformatted"),
                        Action::Check | Action::Diff => {
                            say!("{shown}: would be reformatted");
                        }
                    }
                }
                let written = match (settings.action, diff) {
                    _ if stdout_closed => Ok(()),
                    (Action::Check, _) => writeln!(stdout, "Would reformat: {shown}"),
                    (_, Some(diff)) => stdout.write_all(diff.as_bytes()),
                    (_, None) => Ok(()),
                };
                match written {
                    // Whoever reads stdout stopped reading, as `head` does:
                    // the rest goes unprinted, and the run ends as it would
                    // have.
                    Err(e) if e.kind() == io::ErrorKind::BrokenPipe => stdout_closed = true,
                    Err(e) => {
                        say!("error: cannot write to stdout: {e}");
                        return ExitCode::from(2);
                    }
                    Ok(()) => {}
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
    if let Some(gave_up) = verification.as_ref().and_then(|v| v.gave_up.as_ref()) {
        report(reporter, gave_up);
        errors += 1;
    }
    let summary = summary(settings.action, changed, unchanged, refused, errors);
    match reporter.verbosity {
        Verbosity::Quiet => {}
        Verbosity::Normal => say!("{summary}"),
        Verbosity::Verbose => say!("{summary} in {:.2} s", started.elapsed().as_secs_f64()),
    }
    if let Some(verification) = verification
        && reporter.verbosity != Verbosity::Quiet
    {
        say!(
            "{}",
            verify::summary(&verification.counts, verification.unrendered)
        );
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
/// each path that is a directory, sorted, each file once under the first of
/// its spellings. Paths that
/// cannot be read are reported, and counted in the second value.
fn discover(paths: &[PathBuf], reporter: Reporter) -> (Vec<PathBuf>, usize) {
    let mut files = Vec::new();
    let mut errors = 0;
    for path in paths {
        match fs::metadata(path) {
            Ok(meta) if meta.is_dir() => {
                let before = files.len();
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
                // Most likely a mistyped path, or files under other names.
                // With this, a run that finds no file at all says why for
                // each of its paths.
                if files.len() == before {
                    report(
                        reporter,
                        &Diagnostic::warning(
                            "chairlift::walk::empty",
                            format!(
                                "{}: no SQL files found in this directory (`*.sql`, `*.sql.j2`, \
                                 `*.j2.sql`, not hidden or ignored)",
                                path.display()
                            ),
                        ),
                    );
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
    // The same file under two spellings, such as `d` and `./d/a.sql`, would
    // be formatted and written twice at once.
    let mut seen = std::collections::HashSet::new();
    files.retain(|file| seen.insert(fs::canonicalize(file).unwrap_or_else(|_| file.clone())));
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
                write_atomically(path, before, after).map_err(|e| match e {
                    WriteError::Changed => FileError::Changed,
                    WriteError::Directory(e) => FileError::Write {
                        error: e,
                        directory: true,
                    },
                    WriteError::Io(e) => FileError::Write {
                        error: e,
                        directory: false,
                    },
                })?;
            }
            (Action::Diff, _) => diff = Some(unified_diff(name, before, after)),
            (Action::Write | Action::Check, _) => {}
        }
    }
    Ok(Outcome { changed, diff })
}

/// Why [`write_atomically`] did not replace a file.
enum WriteError {
    /// The file no longer holds the text it was formatted from.
    Changed,
    /// The file's directory refused the temporary file or the rename.
    Directory(io::Error),
    Io(io::Error),
}

/// An error from making or renaming the temporary file, which only the
/// directory's permissions decide.
fn in_directory(e: io::Error) -> WriteError {
    if e.kind() == io::ErrorKind::PermissionDenied {
        WriteError::Directory(e)
    } else {
        WriteError::Io(e)
    }
}

impl From<io::Error> for WriteError {
    fn from(e: io::Error) -> Self {
        WriteError::Io(e)
    }
}

/// Temporary files [`write_atomically`] has made.
static TEMP_FILES: AtomicUsize = AtomicUsize::new(0);

/// A file that is not found any more has changed since it was read.
fn gone(e: io::Error) -> WriteError {
    if e.kind() == io::ErrorKind::NotFound {
        WriteError::Changed
    } else {
        WriteError::Io(e)
    }
}

/// Replaces the file at `path`, which was read as `before`, with `contents`
/// so that a crash leaves either the old file or the new one, never a
/// truncated one: the text goes to a temporary file in the same directory,
/// is synced, and is renamed over the original. A symbolic link is followed,
/// so the file it points at is replaced and the link stays. The new file
/// gets the old one's permissions.
///
/// Right before the rename the file is read again, and left alone if it no
/// longer holds `before`: with `--verify`, minutes can pass between reading
/// and writing, and an edit made meanwhile must not be lost.
///
/// Renaming needs write permission on the directory only, so a file the
/// user may not write is refused first, as writing it in place would be.
fn write_atomically(path: &Path, before: &str, contents: &str) -> Result<(), WriteError> {
    let target = fs::canonicalize(path).map_err(gone)?;
    fs::OpenOptions::new().write(true).open(&target)?;
    let dir = target
        .parent()
        .ok_or_else(|| io::Error::other("the file has no parent directory"))?;
    // Unique to this call, and short whatever the file's name is.
    let temp = dir.join(format!(
        ".chairlift-{}-{}.tmp",
        std::process::id(),
        TEMP_FILES.fetch_add(1, Ordering::Relaxed)
    ));
    let permissions = fs::metadata(&target)?.permissions();
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    // Created with no more access than the file has, so that the text of a
    // file only its owner may read is never readable by others.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(
        &mut options,
        std::os::unix::fs::PermissionsExt::mode(&permissions),
    );
    let mut file = options.open(&temp).map_err(in_directory)?;
    let result = (|| {
        file.set_permissions(permissions)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        if fs::read(&target).map_err(gone)? != before.as_bytes() {
            return Err(WriteError::Changed);
        }
        fs::rename(&temp, &target).map_err(in_directory)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// The unified diff from `before` to `after`.
///
/// Myers's algorithm takes time growing with the product of the size and
/// the number of differences: for a megabyte whose every statement is
/// relaid, half a minute. Past the deadline the diff is coarser, never
/// wrong; a file of ordinary size never gets near it.
fn unified_diff(name: &str, before: &str, after: &str) -> String {
    unified_diff_within(name, before, after, std::time::Duration::from_millis(500))
}

/// The unified diff from `before` to `after`, as fine as `deadline` allows.
fn unified_diff_within(
    name: &str,
    before: &str,
    after: &str,
    deadline: std::time::Duration,
) -> String {
    // Lines end at `\n` only, as `patch` and `git apply` read them.
    // `similar`'s own lines and its unified diff also end lines at a lone
    // `\r`, which gives hunk ranges and a missing "No newline" marker that
    // `patch` rejects.
    let before: Vec<&str> = before.split_inclusive('\n').collect();
    let after: Vec<&str> = after.split_inclusive('\n').collect();
    let diff = similar::TextDiff::configure()
        .algorithm(similar::Algorithm::Myers)
        .timeout(deadline)
        .diff_slices(&before, &after);
    let mut out = String::new();
    for group in diff.grouped_ops(3) {
        if out.is_empty() {
            out = format!("--- a/{name}\n+++ b/{name}\n");
        }
        let header = similar::udiff::UnifiedHunkHeader::new(&group);
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{header}\n"));
        for change in group.iter().flat_map(|op| diff.iter_changes(op)) {
            out.push(match change.tag() {
                similar::ChangeTag::Equal => ' ',
                similar::ChangeTag::Delete => '-',
                similar::ChangeTag::Insert => '+',
            });
            let line = change.value();
            out.push_str(line);
            if !line.ends_with('\n') {
                out.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
    out
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
          help: this is a bug in chairlift. Please report it at https://github.com/glennib/chairlift/issues with the file, or the smallest part of it that shows the problem, and the output of `chl --version`.
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

    /// The headline and the snippet's header give the same line and column:
    /// columns in `char`s, and a lone `\r` ending a line.
    #[test]
    fn a_position_is_the_same_in_headline_and_snippet() {
        for text in ["select 'æøå', `x\n", "select 1;\rselect 'æ', \"x\n"] {
            let error = chairlift::format(text, &Options::default()).unwrap_err();
            let rendered = render(
                FileError::Format {
                    error,
                    text: text.to_owned(),
                },
                "q.sql",
            );
            let mut place = rendered.split_once("× q.sql:").unwrap().1.split(':');
            let headline = format!("{}:{}", place.next().unwrap(), place.next().unwrap());
            assert!(
                rendered.contains(&format!("╭─[q.sql:{headline}]")),
                "{rendered}"
            );
        }
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

    /// A hunk of a unified diff: where it starts in the input, how many
    /// lines of the input it spans, and its lines with their signs.
    type Hunk = (usize, usize, Vec<(char, String)>);

    /// `before` with the unified diff `diff` applied, checking that every
    /// context and removed line is there.
    fn patch(before: &str, diff: &str) -> String {
        let old: Vec<&str> = before.split_inclusive('\n').collect();
        let mut lines = diff.split_inclusive('\n');
        assert!(lines.next().is_some_and(|l| l.starts_with("--- ")));
        assert!(lines.next().is_some_and(|l| l.starts_with("+++ ")));
        // Each hunk, and its lines with their signs; `\ No newline at end of
        // file` takes the newline off the line before it.
        let mut hunks: Vec<Hunk> = Vec::new();
        for line in lines {
            if let Some(header) = line.strip_prefix("@@ -") {
                let range = header.split(' ').next().unwrap();
                let mut parts = range.split(',');
                let start: usize = parts.next().unwrap().parse().unwrap();
                let len: usize = parts.next().map_or(1, |n| n.parse().unwrap());
                hunks.push((start, len, Vec::new()));
            } else if line.starts_with('\\') {
                let body = &mut hunks.last_mut().unwrap().2;
                assert_eq!(body.last_mut().unwrap().1.pop(), Some('\n'));
            } else {
                let mut chars = line.chars();
                let sign = chars.next().unwrap();
                hunks
                    .last_mut()
                    .unwrap()
                    .2
                    .push((sign, chars.as_str().to_owned()));
            }
        }
        let mut out = String::new();
        let mut at = 0;
        for (start, len, body) in hunks {
            // An empty range starts after its line, a range at its line.
            let first = if len == 0 { start } else { start - 1 };
            assert!(first >= at, "hunks overlap");
            out.extend(old[at..first].iter().copied());
            at = first;
            for (sign, text) in body {
                match sign {
                    ' ' | '-' => {
                        assert_eq!(old[at], text, "line {} of the input", at + 1);
                        at += 1;
                        if sign == ' ' {
                            out.push_str(&text);
                        }
                    }
                    '+' => out.push_str(&text),
                    other => panic!("a diff line starting with {other:?}"),
                }
            }
        }
        out.extend(old[at..].iter().copied());
        out
    }

    /// Past the deadline the diff is coarser, but still turns the input into
    /// the output; with no time at all it is as coarse as it gets.
    #[test]
    fn a_diff_past_its_deadline_still_applies() {
        let unchanged = "SELECT 1;\n".repeat(50);
        let cases = [
            ("select 1;\n", "SELECT 1;\n"),
            ("select 1;", "SELECT 1;\n"),
            (
                &*format!("select a;\n{unchanged}select b;  \n\n\n\n{unchanged}select c"),
                &*format!("SELECT a;\n{unchanged}SELECT b;\n\n\n{unchanged}SELECT c;\n"),
            ),
            (&*format!("{unchanged}select 1;\n"), &*unchanged),
            // A lone `\r` does not end a line for `patch`.
            (
                "select a,\rb from t\nwhere x\n",
                "SELECT\n    a,\n    b\nFROM t\nWHERE x\n",
            ),
            ("select 1\r", "SELECT 1\n"),
        ];
        let mut coarser = 0;
        for (before, after) in cases {
            let diffs = [
                std::time::Duration::ZERO,
                std::time::Duration::from_secs(60),
            ]
            .map(|deadline| unified_diff_within("q.sql", before, after, deadline));
            for diff in &diffs {
                assert_eq!(patch(before, diff), after, "{diff}");
            }
            coarser += usize::from(diffs[0].len() > diffs[1].len());
        }
        assert!(coarser > 0, "no case is diffed past its deadline");
    }
}
