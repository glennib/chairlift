//! `chairlift` — command-line formatter for ClickHouse SQL.
//!
//! Each file is handled on its own by [`process`], which reads, formats and
//! (unless checking) writes it and returns an [`Outcome`] or a [`FileError`];
//! reporting and the exit code are worked out from those afterwards.

use std::fmt;
use std::fs;
use std::io;
use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::LazyLock;

use chairlift::Options;
use chairlift::lexer::Mode;
use clap::Parser;
use clap::ValueEnum;

/// Format ClickHouse SQL and Jinja-templated SQL files.
///
/// Files are formatted in place. Directories are searched for `*.sql`,
/// `*.sql.j2` and `*.j2.sql` files, skipping hidden files and whatever
/// `.gitignore` and `.ignore` files exclude.
#[derive(Debug, Parser)]
#[command(
    version = VERSION.as_str(),
    after_help = "Exit status: 0 when done, 1 when --check or --diff found files that would \
                  change, 2 when a file could not be formatted (unreadable, not UTF-8, a lexer \
                  error, or a failed verification) or the arguments are wrong."
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

const ISSUES: &str = "https://github.com/glennib/chairlift/issues";

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

/// A file that was formatted.
#[derive(Debug)]
struct Outcome {
    /// The file's text, when formatting changed it.
    changed: Option<Changed>,
}

#[derive(Debug)]
struct Changed {
    before: String,
    after: String,
}

/// Why a file was left as it was.
#[derive(Debug)]
enum FileError {
    Read(io::Error),
    NotUtf8 { valid_up_to: usize },
    Format(chairlift::Error),
    Write(io::Error),
}

impl FileError {
    /// The error report for the file called `name`.
    fn report(&self, name: impl fmt::Display) -> String {
        match self {
            FileError::Read(e) => format!("{name}: cannot be read: {e}"),
            FileError::NotUtf8 { valid_up_to } => {
                format!("{name}: not UTF-8 (at byte {valid_up_to}); the file is left unchanged")
            }
            FileError::Format(chairlift::Error::Lex(e)) => {
                format!("{name}:{e}; the file is left unchanged")
            }
            FileError::Format(chairlift::Error::Verify(e)) => format!(
                "{name}: chairlift's output failed verification, so the file is left \
                 unchanged.\nThis is a bug in chairlift. Please report it at {ISSUES} with the \
                 file, or the smallest part of it that shows the problem, and the output of \
                 `chairlift --version`.\n  {e}"
            ),
            FileError::Format(e) => format!("{name}: {e}; the file is left unchanged"),
            FileError::Write(e) => {
                format!("{name}: cannot be written: {e}; the file is left as it was")
            }
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
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
        return stdin(&settings);
    }
    files(&cli.paths, &settings)
}

/// Formats stdin to stdout, or checks or diffs it.
fn stdin(settings: &Settings) -> ExitCode {
    let result = read_stdin().and_then(|source| {
        chairlift::format(&source, &settings.options(None))
            .map(|after| (source, after))
            .map_err(FileError::Format)
    });
    let (before, after) = match result {
        Ok(texts) => texts,
        Err(e) => {
            eprintln!("error: {}", e.report("<stdin>"));
            return ExitCode::from(2);
        }
    };
    let mut stdout = io::stdout().lock();
    let written = match settings.action {
        Action::Write => stdout.write_all(after.as_bytes()),
        Action::Check if before != after => writeln!(stdout, "Would reformat: -"),
        Action::Diff if before != after => {
            stdout.write_all(unified_diff("-", &before, &after).as_bytes())
        }
        Action::Check | Action::Diff => Ok(()),
    };
    if let Err(e) = written.and_then(|()| stdout.flush()) {
        eprintln!("error: cannot write to stdout: {e}");
        return ExitCode::from(2);
    }
    if settings.action != Action::Write && before != after {
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
    String::from_utf8(bytes).map_err(|e| FileError::NotUtf8 {
        valid_up_to: e.utf8_error().valid_up_to(),
    })
}

/// Formats, checks or diffs the files under `paths`.
fn files(paths: &[PathBuf], settings: &Settings) -> ExitCode {
    let (files, mut errors) = discover(paths);
    let mut changed = 0;
    let mut unchanged = 0;
    let mut stdout = io::stdout().lock();
    for path in &files {
        match process(path, settings) {
            Ok(Outcome { changed: None }) => unchanged += 1,
            Ok(Outcome {
                changed: Some(Changed { before, after }),
            }) => {
                changed += 1;
                let shown = path.display().to_string();
                let written = match settings.action {
                    Action::Write => Ok(()),
                    Action::Check => writeln!(stdout, "Would reformat: {shown}"),
                    Action::Diff => {
                        stdout.write_all(unified_diff(&shown, &before, &after).as_bytes())
                    }
                };
                if let Err(e) = written {
                    eprintln!("error: cannot write to stdout: {e}");
                    return ExitCode::from(2);
                }
            }
            Err(e) => {
                eprintln!("error: {}", e.report(path.display()));
                errors += 1;
            }
        }
    }
    drop(stdout);
    eprintln!("{}", summary(settings.action, changed, unchanged, errors));
    if errors > 0 {
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
fn discover(paths: &[PathBuf]) -> (Vec<PathBuf>, usize) {
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
                            eprintln!("error: {e}");
                            errors += 1;
                        }
                    }
                }
            }
            Ok(_) => files.push(path.clone()),
            Err(e) => {
                eprintln!("error: {}", FileError::Read(e).report(path.display()));
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
    let bytes = fs::read(path).map_err(FileError::Read)?;
    let before = String::from_utf8(bytes).map_err(|e| FileError::NotUtf8 {
        valid_up_to: e.utf8_error().valid_up_to(),
    })?;
    let after =
        chairlift::format(&before, &settings.options(Some(path))).map_err(FileError::Format)?;
    if after == before {
        return Ok(Outcome { changed: None });
    }
    if settings.action == Action::Write {
        write_atomically(path, &after).map_err(FileError::Write)?;
    }
    Ok(Outcome {
        changed: Some(Changed { before, after }),
    })
}

/// Replaces the file at `path` with `contents` so that a crash leaves either
/// the old file or the new one, never a truncated one: the text goes to a
/// temporary file in the same directory, is synced, and is renamed over the
/// original. A symbolic link is followed, so the file it points at is
/// replaced and the link stays.
fn write_atomically(path: &Path, contents: &str) -> io::Result<()> {
    let target = fs::canonicalize(path)?;
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

fn summary(action: Action, changed: usize, unchanged: usize, errors: usize) -> String {
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

    #[test]
    fn a_verification_failure_asks_for_a_bug_report() {
        let error = FileError::Format(chairlift::Error::Verify(chairlift::verify::VerifyError {
            input: chairlift::Position::new("a -1", 2),
            output: chairlift::Position::new("a--1", 1),
            detail: "token 2 differs".to_owned(),
        }));
        let message = error.report("q.sql");
        assert!(message.starts_with(
            "q.sql: chairlift's output failed verification, so the file is left unchanged."
        ));
        assert!(message.contains(ISSUES));
        assert!(message.ends_with("input 1:3, output 1:2: token 2 differs"));
    }

    #[test]
    fn summaries() {
        assert_eq!(
            summary(Action::Write, 1, 0, 0),
            "1 file reformatted, 0 files already formatted"
        );
        assert_eq!(
            summary(Action::Check, 2, 3, 1),
            "2 files would be reformatted, 3 files already formatted, \
             1 error"
        );
    }
}
