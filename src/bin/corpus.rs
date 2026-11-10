//! `corpus` — runs chairlift over directory trees of SQL and reports what
//! happened to every file: whether it formatted, verified and was idempotent,
//! how long it took, and optionally what `clickhouse-format` makes of input
//! and output (see `chairlift::oracle`).
//!
//! A development tool behind the `corpus-tool` feature:
//!
//! ```text
//! cargo run --release --features corpus-tool --bin corpus -- --default --oracle --csv out.csv
//! ```
//!
//! It prints a summary table per corpus, lists every file that shows a
//! chairlift bug (a verification failure, output that is not idempotent, or
//! an oracle difference that is not allowlisted) and exits with 1 if there
//! is one. `--csv` writes one row per file for analysis, for example with
//! `duckdb -c "SELECT corpus, outcome, count(*) FROM 'out.csv' GROUP BY ALL"`.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Instant;

use chairlift::Error;
use chairlift::Options;
use chairlift::lexer::Mode;
use chairlift::oracle;
use chairlift::oracle::Judgement;
use chairlift::oracle::Oracle;
use chairlift::oracle::RejectionClass;
use chairlift::split::split;
use clap::Parser;

/// Format, verify and check whole corpora of SQL files.
#[derive(Debug, Parser)]
struct Cli {
    /// A directory whose files are templates when their names say so
    /// (`*.sql.j2`, `*.j2.sql`), as `NAME=DIR` or `DIR`.
    #[arg(long = "corpus", value_name = "[NAME=]DIR")]
    corpora: Vec<String>,

    /// A directory whose files are all templates, whatever their names, as
    /// dbt models are.
    #[arg(long = "templates", value_name = "[NAME=]DIR")]
    templates: Vec<String>,

    /// The standard corpora: the vendored fixtures; the ClickHouse stateless
    /// tests and dbt projects in `$CHAIRLIFT_CORPUS_DIR` (default `corpus/`);
    /// and the directories in `$CHAIRLIFT_LOCAL_CORPORA` (`:`-separated). A
    /// missing fetched corpus is skipped, or an error when
    /// `$CHAIRLIFT_REQUIRE_CORPUS` is set.
    #[arg(long)]
    default: bool,

    /// Also run `clickhouse-format` on every input and output.
    #[arg(long)]
    oracle: bool,

    /// The `clickhouse-format` binary.
    #[arg(long, value_name = "PATH", default_value = "clickhouse-format")]
    clickhouse_format: PathBuf,

    /// Files whose oracle differences are expected, one `CORPUS/PATH` per
    /// line; `#` starts a comment.
    #[arg(long, value_name = "FILE")]
    allowlist: Option<PathBuf>,

    /// Write one row per file to this CSV file.
    #[arg(long, value_name = "FILE")]
    csv: Option<PathBuf>,

    /// Files formatted at once [default: the number of CPUs].
    #[arg(long, value_name = "N")]
    jobs: Option<usize>,
}

struct Corpus {
    name: String,
    root: PathBuf,
    all_templates: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Formatted,
    NotUtf8,
    LexError,
    VerifyError,
    TooLarge,
    ReadError,
}

impl Outcome {
    const fn name(self) -> &'static str {
        match self {
            Outcome::Formatted => "formatted",
            Outcome::NotUtf8 => "not_utf8",
            Outcome::LexError => "lex_error",
            Outcome::VerifyError => "verify_error",
            Outcome::TooLarge => "too_large",
            Outcome::ReadError => "read_error",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OracleOutcome {
    Same,
    Differs,
    OutputRejected,
    Allowlisted,
    Unjudgeable(RejectionClass),
}

impl OracleOutcome {
    const fn name(self) -> &'static str {
        match self {
            OracleOutcome::Same => "same",
            OracleOutcome::Differs => "differs",
            OracleOutcome::OutputRejected => "output_rejected",
            OracleOutcome::Allowlisted => "allowlisted",
            OracleOutcome::Unjudgeable(_) => "unjudgeable",
        }
    }
}

/// What happened to one file.
struct Record {
    corpus: usize,
    /// Relative to the corpus root, with `/` separators.
    path: String,
    mode: Mode,
    bytes: usize,
    tokens: usize,
    statements: usize,
    outcome: Outcome,
    changed: Option<bool>,
    idempotent: Option<bool>,
    format_us: u128,
    oracle: Option<OracleOutcome>,
    detail: String,
    /// The input and output, kept for the oracle.
    texts: Option<(String, String)>,
}

impl Record {
    fn is_bug(&self) -> bool {
        self.outcome == Outcome::VerifyError
            || self.idempotent == Some(false)
            || matches!(
                self.oracle,
                Some(OracleOutcome::Differs | OracleOutcome::OutputRejected)
            )
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

/// Runs everything; `Ok(false)` when a chairlift bug was found.
fn run(cli: &Cli) -> Result<bool, String> {
    let corpora = corpora(cli)?;
    if corpora.is_empty() {
        return Err("no corpora given; pass --default, --corpus or --templates".to_owned());
    }
    let allowlist = match &cli.allowlist {
        Some(path) => read_allowlist(path)?,
        None => BTreeSet::new(),
    };
    if let Some(path) = &cli.csv {
        check_writable(path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    let jobs = cli
        .jobs
        .unwrap_or_else(|| thread::available_parallelism().map_or(1, usize::from))
        .max(1);

    let started = Instant::now();
    let mut files = Vec::new();
    for (i, corpus) in corpora.iter().enumerate() {
        for path in sql_files(&corpus.root) {
            files.push((i, path));
        }
    }
    let mut records = format_all(&corpora, &files, jobs);
    let format_time = started.elapsed();

    let oracle_time = if cli.oracle {
        let started = Instant::now();
        let mut oracle = Oracle::new(&cli.clickhouse_format);
        oracle.jobs = jobs;
        if !oracle.is_available() {
            return Err(format!(
                "{} does not run; install it with `mise install` or pass --clickhouse-format",
                cli.clickhouse_format.display()
            ));
        }
        judge_all(&oracle, &corpora, &mut records, &allowlist);
        Some(started.elapsed())
    } else {
        None
    };

    print!("{}", summary(&corpora, &records, cli.oracle));
    println!(
        "{} files formatted in {:.2} s{}",
        records.len(),
        format_time.as_secs_f64(),
        oracle_time.map_or_else(String::new, |t| format!(
            "; oracle in {:.2} s",
            t.as_secs_f64()
        ))
    );
    let mut ok = true;
    for record in records.iter().filter(|r| r.is_bug()) {
        ok = false;
        eprintln!(
            "bug: {}/{}: {}",
            corpora[record.corpus].name, record.path, record.detail
        );
    }
    if let Some(path) = &cli.csv {
        fs::write(path, csv(&corpora, &records))
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(ok)
}

/// The corpora the arguments name.
fn corpora(cli: &Cli) -> Result<Vec<Corpus>, String> {
    let mut corpora = Vec::new();
    let named = |arg: &str, all_templates| {
        let (name, dir) = if let Some((name, dir)) = arg.split_once('=') {
            (name.to_owned(), PathBuf::from(dir))
        } else {
            let dir = PathBuf::from(arg);
            let name = dir
                .file_name()
                .map_or_else(|| arg.to_owned(), |n| n.to_string_lossy().into_owned());
            (name, dir)
        };
        if dir.is_dir() {
            Ok(Corpus {
                name,
                root: dir,
                all_templates,
            })
        } else {
            Err(format!("{} is not a directory", dir.display()))
        }
    };
    for arg in &cli.corpora {
        corpora.push(named(arg, false)?);
    }
    for arg in &cli.templates {
        corpora.push(named(arg, true)?);
    }
    if cli.default {
        corpora.extend(default_corpora()?);
    }
    Ok(corpora)
}

fn default_corpora() -> Result<Vec<Corpus>, String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fetched = std::env::var_os("CHAIRLIFT_CORPUS_DIR")
        .map_or_else(|| manifest.join("corpus"), PathBuf::from);
    let require = std::env::var_os("CHAIRLIFT_REQUIRE_CORPUS").is_some_and(|v| !v.is_empty());
    let mut corpora = vec![Corpus {
        name: "fixtures".to_owned(),
        root: manifest.join("tests/fixtures"),
        all_templates: false,
    }];
    for (name, dir, all_templates) in [
        (
            "stateless",
            fetched.join("ClickHouse/tests/queries/0_stateless"),
            false,
        ),
        ("dbt", fetched.join("dbt"), true),
    ] {
        if dir.is_dir() {
            corpora.push(Corpus {
                name: name.to_owned(),
                root: dir,
                all_templates,
            });
        } else if require {
            return Err(format!(
                "{} is missing, and CHAIRLIFT_REQUIRE_CORPUS is set",
                dir.display()
            ));
        } else {
            eprintln!("skipping: {} is missing", dir.display());
        }
    }
    if let Some(local) = std::env::var_os("CHAIRLIFT_LOCAL_CORPORA") {
        let dirs = std::env::split_paths(&local).filter(|d| d.is_dir());
        for (i, root) in dirs.enumerate() {
            corpora.push(Corpus {
                name: format!("local-{}", i + 1),
                root,
                all_templates: false,
            });
        }
    }
    Ok(corpora)
}

/// The `*.sql` and `*.sql.j2` files under `root`, sorted, outside `.git` and
/// `target` directories. Ignore files do not apply: a corpus is what it is.
fn sql_files(root: &Path) -> Vec<PathBuf> {
    let walk = ignore::WalkBuilder::new(root)
        .standard_filters(false)
        .filter_entry(|e| e.file_name() != ".git" && e.file_name() != "target")
        .build();
    let mut files: Vec<PathBuf> = walk
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(ignore::DirEntry::into_path)
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                Path::new(n).extension().is_some_and(|e| e == "sql") || n.ends_with(".sql.j2")
            })
        })
        .collect();
    files.sort();
    files
}

fn read_allowlist(path: &Path) -> Result<BTreeSet<String>, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(text
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Formats every file, `jobs` at a time.
fn format_all(corpora: &[Corpus], files: &[(usize, PathBuf)], jobs: usize) -> Vec<Record> {
    let next = AtomicUsize::new(0);
    let records = Mutex::new(Vec::with_capacity(files.len()));
    thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((corpus, path)) = files.get(i) else {
                        break;
                    };
                    let record = format_file(*corpus, &corpora[*corpus], path);
                    records
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push((i, record));
                }
            });
        }
    });
    let mut records = records
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    records.sort_by_key(|(i, _)| *i);
    records.into_iter().map(|(_, r)| r).collect()
}

fn format_file(index: usize, corpus: &Corpus, path: &Path) -> Record {
    let relative = path.strip_prefix(&corpus.root).unwrap_or(path);
    let mode = if corpus.all_templates {
        Mode::Jinja
    } else {
        Mode::for_path(path)
    };
    let mut record = Record {
        corpus: index,
        path: relative.to_string_lossy().replace('\\', "/"),
        mode,
        bytes: 0,
        tokens: 0,
        statements: 0,
        outcome: Outcome::Formatted,
        changed: None,
        idempotent: None,
        format_us: 0,
        oracle: None,
        detail: String::new(),
        texts: None,
    };
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            record.outcome = Outcome::ReadError;
            record.detail = e.to_string();
            return record;
        }
    };
    record.bytes = bytes.len();
    let source = match String::from_utf8(bytes) {
        Ok(source) => source,
        Err(e) => {
            record.outcome = Outcome::NotUtf8;
            record.detail = format!("not UTF-8 at byte {}", e.utf8_error().valid_up_to());
            return record;
        }
    };
    let options = Options::default().with_jinja(mode == Mode::Jinja);
    let started = Instant::now();
    let result = chairlift::format(&source, &options);
    record.format_us = started.elapsed().as_micros();
    if u32::try_from(source.len()).is_ok() {
        let split = split(&source, mode);
        record.tokens = split.tokens().len();
        record.statements = split.statements().len();
    }
    match result {
        Ok(output) => {
            record.changed = Some(output != source);
            match chairlift::format(&output, &options) {
                Ok(again) if again == output => record.idempotent = Some(true),
                Ok(_) => {
                    record.idempotent = Some(false);
                    "formatting the output changes it again".clone_into(&mut record.detail);
                }
                Err(e) => {
                    record.idempotent = Some(false);
                    record.detail = format!("formatting the output fails: {e}");
                }
            }
            record.texts = Some((source, output));
        }
        Err(e) => {
            record.outcome = match e {
                Error::Lex(_) => Outcome::LexError,
                Error::Verify(_) => Outcome::VerifyError,
                _ => Outcome::TooLarge,
            };
            record.detail = e.to_string();
        }
    }
    record
}

/// Runs the oracle on the input and output of every formatted file.
fn judge_all(
    oracle: &Oracle,
    corpora: &[Corpus],
    records: &mut [Record],
    allowlist: &BTreeSet<String>,
) {
    let mut texts = Vec::new();
    let mut slots = Vec::new();
    for record in records.iter() {
        let Some((input, output)) = &record.texts else {
            slots.push(None);
            continue;
        };
        let input_slot = texts.len();
        texts.push(oracle::prepare(input, record.mode));
        let output_slot = if input == output {
            input_slot
        } else {
            texts.push(oracle::prepare(output, record.mode));
            texts.len() - 1
        };
        slots.push(Some((input_slot, output_slot)));
    }
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let verdicts = oracle.canonical(&refs);
    for (record, slot) in records.iter_mut().zip(slots) {
        let Some((input, output)) = slot else {
            continue;
        };
        let key = format!("{}/{}", corpora[record.corpus].name, record.path);
        record.oracle = Some(match oracle::judge(&verdicts[input], &verdicts[output]) {
            Judgement::Same => OracleOutcome::Same,
            Judgement::Differs { input, output } => {
                record.detail = first_difference(&input, &output);
                if allowlist.contains(&key) {
                    OracleOutcome::Allowlisted
                } else {
                    OracleOutcome::Differs
                }
            }
            Judgement::OutputRejected(why) => {
                record.detail = format!("the oracle rejects the output: {}", why.message);
                OracleOutcome::OutputRejected
            }
            Judgement::Unjudgeable(why) => {
                record.detail = why.message;
                OracleOutcome::Unjudgeable(why.class)
            }
        });
        record.texts = None;
    }
}

/// The first line where two canonical forms differ.
fn first_difference(input: &str, output: &str) -> String {
    let mut a = input.lines();
    let mut b = output.lines();
    loop {
        match (a.next(), b.next()) {
            (Some(x), Some(y)) if x == y => {}
            (x, y) => {
                return format!(
                    "the oracle prints {:?} for the input and {:?} for the output",
                    x.unwrap_or(""),
                    y.unwrap_or("")
                );
            }
        }
    }
}

fn summary(corpora: &[Corpus], records: &[Record], with_oracle: bool) -> String {
    let mut rows = vec![vec![
        "corpus".to_owned(),
        "files".to_owned(),
        "formatted".to_owned(),
        "changed".to_owned(),
        "lexer errors".to_owned(),
        "not UTF-8".to_owned(),
        "verify failures".to_owned(),
        "not idempotent".to_owned(),
        "format ms".to_owned(),
        "p50 µs".to_owned(),
        "p95 µs".to_owned(),
        "max µs".to_owned(),
    ]];
    let mut oracle_rows = vec![vec![
        "corpus".to_owned(),
        "input accepted".to_owned(),
        "same".to_owned(),
        "differs".to_owned(),
        "output rejected".to_owned(),
        "allowlisted".to_owned(),
        "unjudgeable".to_owned(),
        "unrendered_jinja".to_owned(),
        "insert_data".to_owned(),
        "empty_query".to_owned(),
        "syntax".to_owned(),
        "other".to_owned(),
        "failed".to_owned(),
    ]];
    for (i, corpus) in corpora.iter().enumerate() {
        let of: Vec<&Record> = records.iter().filter(|r| r.corpus == i).collect();
        let count = |f: &dyn Fn(&Record) -> bool| of.iter().filter(|r| f(r)).count().to_string();
        let mut times: Vec<u128> = of
            .iter()
            .filter(|r| r.outcome == Outcome::Formatted)
            .map(|r| r.format_us)
            .collect();
        times.sort_unstable();
        let percentile = |p: usize| {
            times
                .get((times.len() * p / 100).min(times.len().saturating_sub(1)))
                .map_or_else(String::new, ToString::to_string)
        };
        rows.push(vec![
            corpus.name.clone(),
            of.len().to_string(),
            count(&|r| r.outcome == Outcome::Formatted),
            count(&|r| r.changed == Some(true)),
            count(&|r| r.outcome == Outcome::LexError),
            count(&|r| r.outcome == Outcome::NotUtf8),
            count(&|r| r.outcome == Outcome::VerifyError),
            count(&|r| r.idempotent == Some(false)),
            (times.iter().sum::<u128>() / 1000).to_string(),
            percentile(50),
            percentile(95),
            times.last().map_or_else(String::new, ToString::to_string),
        ]);
        let oracle_is = |o: OracleOutcome| move |r: &Record| r.oracle == Some(o);
        let unjudgeable =
            |c: RejectionClass| move |r: &Record| r.oracle == Some(OracleOutcome::Unjudgeable(c));
        oracle_rows.push(vec![
            corpus.name.clone(),
            count(&|r| {
                matches!(
                    r.oracle,
                    Some(
                        OracleOutcome::Same
                            | OracleOutcome::Differs
                            | OracleOutcome::OutputRejected
                            | OracleOutcome::Allowlisted
                    )
                )
            }),
            count(&oracle_is(OracleOutcome::Same)),
            count(&oracle_is(OracleOutcome::Differs)),
            count(&oracle_is(OracleOutcome::OutputRejected)),
            count(&oracle_is(OracleOutcome::Allowlisted)),
            count(&|r| matches!(r.oracle, Some(OracleOutcome::Unjudgeable(_)))),
            count(&unjudgeable(RejectionClass::UnrenderedJinja)),
            count(&unjudgeable(RejectionClass::InsertData)),
            count(&unjudgeable(RejectionClass::EmptyQuery)),
            count(&unjudgeable(RejectionClass::Syntax)),
            count(&unjudgeable(RejectionClass::Other)),
            count(&unjudgeable(RejectionClass::Failed)),
        ]);
    }
    let mut out = table(&rows);
    if with_oracle {
        out.push('\n');
        out.push_str(&table(&oracle_rows));
    }
    out
}

/// Rows as an aligned text table, numbers right-aligned.
fn table(rows: &[Vec<String>]) -> String {
    let columns = rows.first().map_or(0, Vec::len);
    let widths: Vec<usize> = (0..columns)
        .map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0))
        .collect();
    let mut out = String::new();
    for (n, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let pad = widths[c] - cell.chars().count();
            if c > 0 {
                out.push_str("  ");
            }
            if c == 0 || n == 0 {
                out.push_str(cell);
                out.extend(std::iter::repeat_n(' ', pad));
            } else {
                out.extend(std::iter::repeat_n(' ', pad));
                out.push_str(cell);
            }
        }
        out.truncate(out.trim_end().len());
        out.push('\n');
    }
    out
}

/// The CSV columns, in order.
const CSV_COLUMNS: &str = "corpus,path,mode,bytes,tokens,statements,outcome,changed,idempotent,\
                           format_us,oracle,oracle_class,detail";

fn csv(corpora: &[Corpus], records: &[Record]) -> String {
    let mut out = String::from(CSV_COLUMNS);
    out.push('\n');
    let flag = |b: Option<bool>| b.map_or("", |b| if b { "true" } else { "false" });
    for r in records {
        let class = match r.oracle {
            Some(OracleOutcome::Unjudgeable(class)) => class.name(),
            Some(OracleOutcome::OutputRejected) => "output",
            _ => "",
        };
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{},{},{},{},{},{}",
            quote(&corpora[r.corpus].name),
            quote(&r.path),
            if r.mode == Mode::Jinja {
                "jinja"
            } else {
                "sql"
            },
            r.bytes,
            r.tokens,
            r.statements,
            r.outcome.name(),
            flag(r.changed),
            flag(r.idempotent),
            r.format_us,
            r.oracle.map_or("", OracleOutcome::name),
            class,
            quote(r.detail.lines().next().unwrap_or("")),
        );
    }
    out
}

/// A CSV field, quoted when it needs to be.
fn quote(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

/// Whether `path` can be written, checked before the work rather than after.
fn check_writable(path: &Path) -> io::Result<()> {
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map(|_| ())
}
