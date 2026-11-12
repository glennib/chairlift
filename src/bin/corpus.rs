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
//! chairlift bug (a verification failure, output that is not idempotent, a
//! syntax tree that does not give back the input, a panic, or an oracle
//! difference that is not allowlisted) and exits with 1 if there is one.
//! `--csv` writes one row per file for analysis, for example with
//! `duckdb -c "SELECT corpus, outcome, count(*) FROM 'out.csv' GROUP BY ALL"`,
//! and `--statements-csv` one row per statement, with how much of it parsed.
//!
//! Every file is also parsed, to report how much of each statement kind
//! parses and why the rest fell back to verbatim. `--recase` replaces the
//! formatter with the parser's keyword re-casing, which `verify` and the
//! oracle then check: `--recase all` re-cases the keyword slots of the whole
//! parse, and `--recase expressions` parses only `SELECT <list>` statements,
//! as expression lists, and re-cases those (see
//! `chairlift::syntax::Entry::SelectListProbe`). `--tree FILE` prints the
//! syntax tree of a file instead.
//!
//! With `--oracle`, files the oracle cannot judge as a whole are judged
//! statement by statement (see `chairlift::oracle::Oracle::judge_pairs`).
//! The second table counts files by how their whole text fared; the third
//! counts statements, which shows how much of each corpus the oracle
//! actually judged. The `oracle_*` CSV columns hold the statement counts per
//! file.
//!
//! Every template that formats is rendered, input and output, once per
//! combination of its branches and loop count (see `chairlift::template`),
//! and the SQL of each pair of renders compared token by token; with
//! `--oracle`, each distinct pair is judged by the oracle too. The templates
//! table counts the variants, how many rendered, and the differences. A
//! difference, or a variant only one side renders, is a chairlift bug.
//! `--only-templates` leaves out the files that are not templates.
//!
//! `--extract-docs` writes the docs-examples corpus: the SQL blocks of the
//! fetched ClickHouse documentation that `clickhouse-format` accepts, as
//! files under `$CHAIRLIFT_CORPUS_DIR/docs` (see `docs.rs`). `mise run
//! corpus:fetch` runs it after the fetch.

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
use chairlift::layout::Decision;
use chairlift::layout::Kept;
use chairlift::lexer::Mode;
use chairlift::oracle;
use chairlift::oracle::Counts;
use chairlift::oracle::Judgement;
use chairlift::oracle::Oracle;
use chairlift::oracle::Pair;
use chairlift::oracle::RejectionClass;
use chairlift::oracle::Report;
use chairlift::split::split;
use chairlift::syntax;
use chairlift::syntax::Entry;
use chairlift::syntax::StatementOutcome;
use chairlift::syntax::SyntaxKind;
use chairlift::syntax::SyntaxNode;
use clap::Parser;

#[path = "corpus/docs.rs"]
mod docs;

/// Format, verify and check whole corpora of SQL files.
#[derive(Debug, Parser)]
#[allow(clippy::struct_excessive_bools, reason = "each is a flag")]
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
    /// tests, dbt projects and docs examples in `$CHAIRLIFT_CORPUS_DIR`
    /// (default `corpus/`);
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

    /// Write one row per statement to this CSV file: its kind, how much of
    /// it parsed, and why the rest did not.
    #[arg(long, value_name = "FILE")]
    statements_csv: Option<PathBuf>,

    /// Instead of formatting, re-case the keyword slots the parser finds,
    /// and check that with `verify` (and the oracle, with `--oracle`):
    /// `all` parses files as the formatter does, `expressions` parses only
    /// `SELECT <list>` statements, as expression lists, and keeps the rest
    /// verbatim, to report how much of the expression grammar parses.
    #[arg(long, value_enum, value_name = "WHAT")]
    recase: Option<Recase>,

    /// Print the syntax tree of each FILE and exit.
    #[arg(long, value_name = "FILE")]
    tree: Vec<PathBuf>,

    /// Files formatted at once [default: the number of CPUs].
    #[arg(long, value_name = "N")]
    jobs: Option<usize>,

    /// Only the files read as templates.
    #[arg(long)]
    only_templates: bool,

    /// Write the docs-examples corpus: the SQL blocks of
    /// `ClickHouse/docs/reference` in `$CHAIRLIFT_CORPUS_DIR` that
    /// `clickhouse-format` accepts, into `docs` there, and exit. Does nothing
    /// when the corpus is already extracted from the same revision with the
    /// same `clickhouse-format`.
    #[arg(long, conflicts_with_all = ["default", "corpora", "templates", "tree"])]
    extract_docs: bool,
}

struct Corpus {
    name: String,
    root: PathBuf,
    all_templates: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum Recase {
    All,
    Expressions,
}

/// What the tool does to each file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transform {
    /// `chairlift::format`.
    Format,
    /// Re-case the keyword slots of the parse as `entry`.
    Recase(Entry),
}

impl Transform {
    /// The entry the statistics parse uses.
    fn entry(self) -> Entry {
        match self {
            Transform::Format => Entry::File,
            Transform::Recase(entry) => entry,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Formatted,
    NotUtf8,
    LexError,
    /// A template whose line comment ends where the rendering decides.
    TemplateComment,
    VerifyError,
    TooLarge,
    ReadError,
    Panic,
}

impl Outcome {
    const fn name(self) -> &'static str {
        match self {
            Outcome::Formatted => "formatted",
            Outcome::NotUtf8 => "not_utf8",
            Outcome::LexError => "lex_error",
            Outcome::TemplateComment => "template_comment",
            Outcome::VerifyError => "verify_error",
            Outcome::TooLarge => "too_large",
            Outcome::ReadError => "read_error",
            Outcome::Panic => "panic",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OracleOutcome {
    Same,
    Differs,
    OutputRejected,
    /// Input and output split into different numbers of statements.
    Mismatch,
    Allowlisted,
    /// The whole input is rejected; its statements may still be judged.
    Unjudgeable(RejectionClass),
}

impl OracleOutcome {
    const fn name(self) -> &'static str {
        match self {
            OracleOutcome::Same => "same",
            OracleOutcome::Differs => "differs",
            OracleOutcome::OutputRejected => "output_rejected",
            OracleOutcome::Mismatch => "statement_mismatch",
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
    /// The statement counts, summed over the file's variants.
    oracle_counts: Option<Counts>,
    /// What went wrong, as one line; the first of `findings` if there are
    /// any.
    detail: String,
    /// Every oracle difference, one line each.
    findings: Vec<String>,
    /// The input and output, kept for the oracle.
    texts: Option<(String, String)>,
    /// What the parse of the input says, if it ran to the end.
    parse: Option<ParseReport>,
    /// The parse panicked, with this message.
    parse_panic: Option<String>,
    /// What the layout did with each statement, when the file formatted.
    layout: Vec<Decision>,
    /// For a template that formatted, what its renders showed.
    renders: Option<Renders>,
}

/// What rendering a template's input and output showed.
#[derive(Clone, Debug, Default)]
struct Renders {
    /// The variants of the input.
    variants: usize,
    /// The variants both texts rendered.
    rendered: usize,
    /// Whether the branch combinations are a sample.
    sampled: bool,
    /// Differences between a render of the input and the output's, a
    /// variant only one of them renders, or branches that differ: bugs.
    differences: Vec<String>,
    /// The oracle's verdicts on the rendered variants.
    oracle: VariantVerdicts,
}

/// The oracle's verdicts on the rendered variants of templates.
#[derive(Clone, Copy, Debug, Default)]
struct VariantVerdicts {
    /// Variants judged.
    judged: usize,
    /// Variants whose rendered input the oracle accepts as a whole.
    accepted: usize,
    /// Of those, the ones whose output it prints the same.
    same: usize,
    /// Variants with a statement or a whole text that differs, or an output
    /// it rejects.
    differs: usize,
    /// Statements of the variants judged one by one.
    statements_judged: usize,
}

impl VariantVerdicts {
    fn add(&mut self, other: &VariantVerdicts) {
        self.judged += other.judged;
        self.accepted += other.accepted;
        self.same += other.same;
        self.differs += other.differs;
        self.statements_judged += other.statements_judged;
    }
}

/// What the parse of one file says.
struct ParseReport {
    /// The tree's text is the input.
    round_trip: bool,
    statements: Vec<StatementRow>,
}

/// What became of one statement.
struct StatementRow {
    kind: String,
    outcome: StatementOutcome,
    clauses: usize,
    verbatim_clauses: usize,
    items: usize,
    verbatim_items: usize,
    /// Its regions kept as written by design.
    opaque: usize,
    /// For a statement the expression probe parsed: how its top-level items
    /// went.
    probe_items: Option<ProbeItems>,
    /// The reason for the statement's first fallback.
    fallback: Option<String>,
    /// What the layout did with it, when the file formatted.
    decision: Option<Decision>,
    /// The start of the statement's text, on one line.
    excerpt: String,
}

/// The top-level items of a statement the expression probe parsed, in the
/// three classes of [`StatementOutcome`]: parsed whole, parsed with regions
/// kept as written by design, and with a fallback inside.
#[derive(Clone, Copy, Debug, Default)]
struct ProbeItems {
    parsed: usize,
    opaque: usize,
    fallback: usize,
}

impl ProbeItems {
    fn all(self) -> usize {
        self.parsed + self.opaque + self.fallback
    }
}

impl Record {
    fn is_bug(&self) -> bool {
        self.outcome == Outcome::VerifyError
            || self.outcome == Outcome::Panic
            || self.parse_panic.is_some()
            || self.parse.as_ref().is_some_and(|p| !p.round_trip)
            || self.idempotent == Some(false)
            || self
                .renders
                .as_ref()
                .is_some_and(|r| !r.differences.is_empty())
            || matches!(
                self.oracle,
                Some(
                    OracleOutcome::Differs
                        | OracleOutcome::OutputRejected
                        | OracleOutcome::Mismatch
                )
            )
    }
}

/// The most findings listed per file; the rest are counted.
const FINDINGS_SHOWN: usize = 5;

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
    if !cli.tree.is_empty() {
        return print_trees(cli);
    }
    if cli.extract_docs {
        return extract_docs(cli);
    }
    let corpora = corpora(cli)?;
    if corpora.is_empty() {
        return Err("no corpora given; pass --default, --corpus or --templates".to_owned());
    }
    let allowlist = match &cli.allowlist {
        Some(path) => read_allowlist(path)?,
        None => BTreeSet::new(),
    };
    for path in cli.csv.iter().chain(&cli.statements_csv) {
        check_writable(path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    let transform = match cli.recase {
        None => Transform::Format,
        Some(Recase::All) => Transform::Recase(Entry::File),
        Some(Recase::Expressions) => Transform::Recase(Entry::SelectListProbe),
    };
    let jobs = cli
        .jobs
        .unwrap_or_else(|| thread::available_parallelism().map_or(1, usize::from))
        .max(1);

    let started = Instant::now();
    let mut files = Vec::new();
    for (i, corpus) in corpora.iter().enumerate() {
        for path in sql_files(&corpus.root) {
            if cli.only_templates && !(corpus.all_templates || Mode::for_path(&path) == Mode::Jinja)
            {
                continue;
            }
            files.push((i, path));
        }
    }
    let mut records = format_all(&corpora, &files, jobs, transform);
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
    print!("{}", parse_summary(&corpora, &records, transform));
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
        let name = format!("{}/{}", corpora[record.corpus].name, record.path);
        if let Some(panic) = &record.parse_panic {
            eprintln!("bug: {name}: the parser panicked: {panic}");
        } else if record.parse.as_ref().is_some_and(|p| !p.round_trip) {
            eprintln!("bug: {name}: the syntax tree's text is not the input");
        } else if record.findings.is_empty() {
            eprintln!("bug: {name}: {}", record.detail);
        }
        if let Some(renders) = &record.renders {
            for difference in renders.differences.iter().take(FINDINGS_SHOWN) {
                eprintln!("bug: {name}: {difference}");
            }
            if renders.differences.len() > FINDINGS_SHOWN {
                eprintln!(
                    "bug: {name}: and {} more render differences",
                    renders.differences.len() - FINDINGS_SHOWN
                );
            }
        }
        for finding in record.findings.iter().take(FINDINGS_SHOWN) {
            eprintln!("bug: {name}: {finding}");
        }
        if record.findings.len() > FINDINGS_SHOWN {
            eprintln!(
                "bug: {name}: and {} more",
                record.findings.len() - FINDINGS_SHOWN
            );
        }
    }
    if let Some(path) = &cli.csv {
        fs::write(path, csv(&corpora, &records))
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    if let Some(path) = &cli.statements_csv {
        fs::write(path, statements_csv(&corpora, &records))
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(ok)
}

/// Prints the syntax tree of each `--tree` file, in the mode its name says.
fn print_trees(cli: &Cli) -> Result<bool, String> {
    for path in &cli.tree {
        let source =
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let entry = if cli.recase == Some(Recase::Expressions) {
            Entry::SelectListProbe
        } else {
            Entry::File
        };
        let parse = syntax::parse_as(&source, Mode::for_path(path), entry);
        print!("{}", syntax::debug_tree(&parse.syntax()));
        for fallback in parse.fallbacks() {
            println!(
                "fallback ({:?}) at {}..{}: {}",
                fallback.level, fallback.range.start, fallback.range.end, fallback.message
            );
        }
    }
    Ok(true)
}

/// The fetched corpora's directory.
fn fetched_dir() -> PathBuf {
    std::env::var_os("CHAIRLIFT_CORPUS_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus"),
        PathBuf::from,
    )
}

/// Writes the docs-examples corpus, `--extract-docs`.
fn extract_docs(cli: &Cli) -> Result<bool, String> {
    let fetched = fetched_dir();
    let pages = fetched.join("ClickHouse/docs/reference");
    let target = fetched.join("docs");
    let mut oracle = Oracle::new(&cli.clickhouse_format);
    if let Some(jobs) = cli.jobs {
        oracle.jobs = jobs.max(1);
    }
    if let Err(e) = oracle.probe() {
        return Err(format!(
            "{e}; install it with `mise install` or pass --clickhouse-format"
        ));
    }
    if !pages.is_dir() {
        return Err(format!(
            "{} is missing; run scripts/fetch-corpus.sh first",
            pages.display()
        ));
    }
    let stamp = docs::stamp(&pages, &cli.clickhouse_format)?;
    let started = Instant::now();
    match docs::extract(&pages, &target, &oracle, &stamp)? {
        None => println!("docs examples in {} are up to date", target.display()),
        Some(stats) => {
            print!("{}", stats.report());
            println!(
                "docs examples written to {} in {:.2} s",
                target.display(),
                started.elapsed().as_secs_f64()
            );
        }
    }
    Ok(true)
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
    let fetched = fetched_dir();
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
        ("docs", fetched.join("docs"), false),
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
fn format_all(
    corpora: &[Corpus],
    files: &[(usize, PathBuf)],
    jobs: usize,
    transform: Transform,
) -> Vec<Record> {
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
                    let record = format_file(*corpus, &corpora[*corpus], path, transform);
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

fn format_file(index: usize, corpus: &Corpus, path: &Path, transform: Transform) -> Record {
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
        oracle_counts: None,
        detail: String::new(),
        findings: Vec::new(),
        texts: None,
        parse: None,
        parse_panic: None,
        layout: Vec::new(),
        renders: None,
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
    let result = apply(&source, &options, transform);
    record.format_us = started.elapsed().as_micros();
    if u32::try_from(source.len()).is_ok() {
        let split = split(&source, mode);
        record.tokens = split.tokens().len();
        record.statements = split.statements().len();
        match std::panic::catch_unwind(|| {
            let parse = syntax::parse_split(&source, &split, transform.entry());
            report(&parse, &source)
        }) {
            Ok(report) => record.parse = Some(report),
            Err(panic) => record.parse_panic = Some(panic_message(&*panic)),
        }
    }
    let result = match result {
        Ok(result) => result,
        Err(panic) => {
            record.outcome = Outcome::Panic;
            record.detail = format!("panicked: {panic}");
            return record;
        }
    };
    match result {
        Ok((output, layout)) => {
            record.changed = Some(output != source);
            if let Some(parse) = &mut record.parse {
                for (row, decision) in parse.statements.iter_mut().zip(&layout) {
                    row.decision = Some(*decision);
                }
            }
            record.layout = layout;
            match apply(&output, &options, transform) {
                Err(panic) => {
                    record.idempotent = Some(false);
                    record.detail = format!("panicked on the output: {panic}");
                }
                Ok(Ok((again, _))) if again == output => record.idempotent = Some(true),
                Ok(Ok(_)) => {
                    record.idempotent = Some(false);
                    "formatting the output changes it again".clone_into(&mut record.detail);
                }
                Ok(Err(e)) => {
                    record.idempotent = Some(false);
                    record.detail = format!("formatting the output fails: {e}");
                }
            }
            if mode == Mode::Jinja && transform == Transform::Format {
                record.renders = Some(compare_renders(&source, &output));
            }
            record.texts = Some((source, output));
        }
        Err(e) => {
            record.outcome = match e {
                Error::Lex(_) => Outcome::LexError,
                Error::Verify(_) | Error::Layout(_) => Outcome::VerifyError,
                Error::Template(_) => Outcome::TemplateComment,
                _ => Outcome::TooLarge,
            };
            record.detail = e.to_string();
        }
    }
    record
}

/// Renders template `input` and its formatted `output` in every variant and
/// compares the SQL of each pair.
fn compare_renders(input: &str, output: &str) -> Renders {
    let before = chairlift::template::render(input);
    let after = chairlift::template::render(output);
    let mut renders = Renders {
        variants: before.variants.len(),
        sampled: before.sampled,
        ..Renders::default()
    };
    if before.branches != after.branches {
        renders.differences.push(format!(
            "the output's branches and loops are {:?}, the input's {:?}",
            after.branches, before.branches
        ));
        return renders;
    }
    for (a, b) in before.variants.iter().zip(&after.variants) {
        match (&a.text, &b.text) {
            (Ok(x), Ok(y)) => {
                renders.rendered += 1;
                if let Err(e) = chairlift::template::compare_sql(x, y) {
                    renders.differences.push(format!("render {}: {e}", a.label));
                }
            }
            (Err(_), Err(_)) => {}
            (Ok(_), Err(e)) => renders
                .differences
                .push(format!("render {}: only the input renders: {e}", a.label)),
            (Err(e), Ok(_)) => renders
                .differences
                .push(format!("render {}: only the output renders: {e}", a.label)),
        }
    }
    renders
}

/// `transform` applied to `source`: formatted, with what the layout did
/// with each statement, or re-cased and verified. `Err` holds a panic's
/// message.
fn apply(
    source: &str,
    options: &Options,
    transform: Transform,
) -> Result<Result<(String, Vec<Decision>), Error>, String> {
    std::panic::catch_unwind(|| match transform {
        Transform::Format => chairlift::format_with_statements(source, options)
            .map(|(output, statements)| (output, statements.iter().map(|s| s.decision).collect())),
        Transform::Recase(entry) => recase(source, options.mode(), entry).map(|o| (o, Vec::new())),
    })
    .map_err(|panic| panic_message(&*panic))
}

/// The keyword slots of `source` re-cased, verified against `source`.
fn recase(source: &str, mode: Mode, entry: Entry) -> Result<String, Error> {
    if u32::try_from(source.len()).is_err() {
        return Err(Error::TooLarge { len: source.len() });
    }
    let split = split(source, mode);
    if let Some(token) = split.first_error() {
        return Err(chairlift::LexError {
            position: chairlift::Position::new(source, token.start()),
            kind: token.kind(),
        }
        .into());
    }
    let parse = syntax::parse_split(source, &split, entry);
    let output = parse.recased();
    chairlift::verify::verify_split(source, &split, &output, mode, parse.facts())?;
    Ok(output)
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic".to_owned())
}

/// What `parse` of `source` says about each statement.
fn report(parse: &syntax::Parse, source: &str) -> ParseReport {
    let root = parse.syntax();
    let round_trip = root.text() == source;
    let fallbacks = parse.fallbacks();
    let statements = root
        .children()
        .filter(|n| n.kind() == SyntaxKind::Statement)
        .zip(parse.statements())
        .map(|(node, stats)| {
            let fallback = fallbacks
                .iter()
                .find(|f| stats.range.contains(&f.range.start))
                .map(|f| f.message.clone());
            StatementRow {
                kind: stats.kind,
                outcome: stats.outcome,
                clauses: stats.clauses,
                verbatim_clauses: stats.verbatim_clauses,
                items: stats.items,
                verbatim_items: stats.verbatim_items,
                opaque: stats.opaque,
                probe_items: probe_items(&node),
                fallback,
                decision: None,
                excerpt: excerpt(&source[stats.range.clone()]),
            }
        })
        .collect();
    ParseReport {
        round_trip,
        statements,
    }
}

/// The start of `text` with its whitespace runs as single spaces, at most
/// 200 characters.
fn excerpt(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
        if out.chars().count() > 200 {
            return out.chars().take(200).chain(std::iter::once('…')).collect();
        }
    }
    out
}

/// For a statement the expression probe parsed, how its top-level items
/// went.
fn probe_items(statement: &SyntaxNode) -> Option<ProbeItems> {
    let probe = statement
        .first_child()
        .filter(|n| n.kind() == SyntaxKind::ExpressionProbe)?;
    let list = probe
        .children()
        .find(|n| n.kind() == SyntaxKind::ExprList)?;
    let mut items = ProbeItems::default();
    for item in list.children() {
        let inside = |test: fn(SyntaxKind) -> bool| item.descendants().any(|n| test(n.kind()));
        if inside(|k| k.fallback_level().is_some()) {
            items.fallback += 1;
        } else if inside(|k| k == SyntaxKind::Opaque) {
            items.opaque += 1;
        } else {
            items.parsed += 1;
        }
    }
    Some(items)
}

/// Runs the oracle on the input and output of every formatted file.
fn judge_all(
    oracle: &Oracle,
    corpora: &[Corpus],
    records: &mut [Record],
    allowlist: &BTreeSet<String>,
) {
    let mut owners = Vec::new();
    let mut labels = Vec::new();
    let mut texts = Vec::new();
    for (r, record) in records.iter().enumerate() {
        if let Some((input, output)) = &record.texts {
            for variant in oracle::variants(input, output, record.mode) {
                owners.push(r);
                labels.push(variant.rendering.unwrap_or_default());
                texts.push((variant.input, variant.output, variant.mode));
            }
        }
    }
    let pairs: Vec<Pair<'_>> = texts
        .iter()
        .map(|(input, output, mode)| Pair {
            input,
            output,
            mode: *mode,
        })
        .collect();
    let reports = oracle.judge_pairs(&pairs);
    drop(pairs);
    drop(texts);

    let mut by_record: Vec<Vec<(String, Report)>> = records.iter().map(|_| Vec::new()).collect();
    for ((r, label), report) in owners.into_iter().zip(labels).zip(reports) {
        by_record[r].push((label, report));
    }
    for (record, reports) in records.iter_mut().zip(by_record) {
        record.texts = None;
        if reports.is_empty() {
            continue;
        }
        let key = format!("{}/{}", corpora[record.corpus].name, record.path);
        // The counts are the file's own; the rendered variants are counted
        // apart.
        let mut counts = Counts::default();
        for (label, report) in &reports {
            if label.is_empty() {
                counts.add(&report.counts());
            } else if let Some(renders) = &mut record.renders {
                renders.oracle.add(&verdicts(report));
            }
        }
        record.oracle_counts = Some(counts);
        record.findings = reports
            .iter()
            .flat_map(|(label, report)| findings(label, report))
            .collect();
        let any = |f: &dyn Fn(&Judgement) -> bool| {
            reports
                .iter()
                .any(|(_, r)| f(&r.whole) || r.statements.iter().any(|s| f(&s.judgement)))
        };
        let outcome = if any(&|j| matches!(j, Judgement::Differs { .. })) {
            if allowlist.contains(&key) {
                OracleOutcome::Allowlisted
            } else {
                OracleOutcome::Differs
            }
        } else if any(&|j| matches!(j, Judgement::OutputRejected(_))) {
            OracleOutcome::OutputRejected
        } else if reports.iter().any(|(_, r)| r.mismatch.is_some()) {
            OracleOutcome::Mismatch
        } else if let Some(why) = reports.iter().find_map(|(_, r)| match &r.whole {
            Judgement::Unjudgeable(why) => Some(why),
            _ => None,
        }) {
            why.message.clone_into(&mut record.detail);
            OracleOutcome::Unjudgeable(why.class)
        } else {
            OracleOutcome::Same
        };
        if let Some(first) = record.findings.first() {
            first.clone_into(&mut record.detail);
        }
        record.oracle = Some(outcome);
    }
}

/// The oracle's verdicts on one rendered variant.
fn verdicts(report: &Report) -> VariantVerdicts {
    let accepted = !matches!(report.whole, Judgement::Unjudgeable(_));
    let differs = matches!(
        report.whole,
        Judgement::Differs { .. } | Judgement::OutputRejected(_)
    ) || report.mismatch.is_some()
        || report.statements.iter().any(|s| {
            matches!(
                s.judgement,
                Judgement::Differs { .. } | Judgement::OutputRejected(_)
            )
        });
    VariantVerdicts {
        judged: 1,
        accepted: usize::from(accepted),
        same: usize::from(accepted && matches!(report.whole, Judgement::Same)),
        differs: usize::from(differs),
        statements_judged: report.counts().judged(),
    }
}

/// One line per difference that `report` shows, each prefixed with `label`.
fn findings(label: &str, report: &Report) -> Vec<String> {
    let prefix = if label.is_empty() {
        String::new()
    } else {
        format!("{label}: ")
    };
    let mut out = Vec::new();
    if let Some(mismatch) = report.mismatch {
        out.push(format!(
            "{prefix}the input splits into {} statements and the output into {}",
            mismatch.input, mismatch.output
        ));
    }
    for (n, statement) in report.statements.iter().enumerate() {
        let at = format!(
            "{prefix}statement {} (line {}, output line {})",
            n + 1,
            statement.line,
            statement.output_line
        );
        match &statement.judgement {
            Judgement::Differs { input, output } => {
                out.push(format!("{at}: {}", oracle::difference(input, output)));
            }
            Judgement::OutputRejected(why) => {
                out.push(format!(
                    "{at}: the oracle rejects the output: {}",
                    why.message
                ));
            }
            Judgement::Same | Judgement::Unjudgeable(_) => {}
        }
    }
    // A difference between the whole texts that no single statement shows.
    if out.is_empty() {
        match &report.whole {
            Judgement::Differs { input, output } => out.push(format!(
                "{prefix}the whole text, though no statement on its own: {}",
                oracle::difference(input, output)
            )),
            Judgement::OutputRejected(why) => out.push(format!(
                "{prefix}the whole text, though no statement on its own: the oracle rejects \
                 the output: {}",
                why.message
            )),
            Judgement::Same | Judgement::Unjudgeable(_) => {}
        }
    }
    out
}

fn summary(corpora: &[Corpus], records: &[Record], with_oracle: bool) -> String {
    let mut rows = vec![vec![
        "corpus".to_owned(),
        "files".to_owned(),
        "formatted".to_owned(),
        "changed".to_owned(),
        "lexer errors".to_owned(),
        "templates refused".to_owned(),
        "not UTF-8".to_owned(),
        "verify failures".to_owned(),
        "not idempotent".to_owned(),
        "format ms".to_owned(),
        "p50 µs".to_owned(),
        "p95 µs".to_owned(),
        "max µs".to_owned(),
    ]];
    let header = |names: &[&str]| names.iter().map(|&n| n.to_owned()).collect::<Vec<_>>();
    let mut file_rows = vec![header(&[
        "corpus",
        "files",
        "same",
        "differs",
        "output rejected",
        "statement mismatch",
        "allowlisted",
        "unjudgeable",
    ])];
    file_rows[0].extend(RejectionClass::ALL.map(|c| c.name().to_owned()));
    let mut statement_rows = vec![header(&[
        "corpus",
        "statements",
        "judged",
        "judged %",
        "same",
        "differs",
        "output rejected",
    ])];
    statement_rows[0].extend(RejectionClass::ALL.map(|c| c.name().to_owned()));
    statement_rows[0].extend(header(&["changed", "changed judged"]));
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
            count(&|r| r.outcome == Outcome::TemplateComment),
            count(&|r| r.outcome == Outcome::NotUtf8),
            count(&|r| r.outcome == Outcome::VerifyError),
            count(&|r| r.idempotent == Some(false)),
            (times.iter().sum::<u128>() / 1000).to_string(),
            percentile(50),
            percentile(95),
            times.last().map_or_else(String::new, ToString::to_string),
        ]);

        let oracle_is = |o: OracleOutcome| move |r: &Record| r.oracle == Some(o);
        let mut row = vec![
            corpus.name.clone(),
            count(&|r| r.oracle.is_some()),
            count(&oracle_is(OracleOutcome::Same)),
            count(&oracle_is(OracleOutcome::Differs)),
            count(&oracle_is(OracleOutcome::OutputRejected)),
            count(&oracle_is(OracleOutcome::Mismatch)),
            count(&oracle_is(OracleOutcome::Allowlisted)),
            count(&|r| matches!(r.oracle, Some(OracleOutcome::Unjudgeable(_)))),
        ];
        row.extend(RejectionClass::ALL.map(|c| count(&oracle_is(OracleOutcome::Unjudgeable(c)))));
        file_rows.push(row);

        let mut sum = Counts::default();
        for counts in of.iter().filter_map(|r| r.oracle_counts.as_ref()) {
            sum.add(counts);
        }
        let mut row = vec![
            corpus.name.clone(),
            sum.statements.to_string(),
            sum.judged().to_string(),
            percent(sum.judged(), sum.statements),
            sum.same.to_string(),
            sum.differs.to_string(),
            sum.output_rejected.to_string(),
        ];
        row.extend(RejectionClass::ALL.map(|c| sum.rejected(c).to_string()));
        row.extend([sum.changed.to_string(), sum.changed_judged.to_string()]);
        statement_rows.push(row);
    }
    let mut out = table(&rows);
    out.push_str(&templates_summary(corpora, records, with_oracle));
    if with_oracle {
        out.push_str("\nfiles, by the oracle's verdict on the whole text\n");
        out.push_str(&table(&file_rows));
        out.push_str("\nstatements, by the oracle's verdict on each\n");
        out.push_str(&table(&statement_rows));
    }
    out
}

/// The renders of templates per corpus: how many variants rendered, how many
/// differ, and with the oracle what it made of them.
fn templates_summary(corpora: &[Corpus], records: &[Record], with_oracle: bool) -> String {
    let mut rows = vec![
        [
            "corpus",
            "templates",
            "sampled",
            "variants",
            "rendered",
            "render failures",
            "differences",
        ]
        .map(str::to_owned)
        .to_vec(),
    ];
    if with_oracle {
        rows[0].extend(
            [
                "oracle judged",
                "accepted whole",
                "same",
                "differing",
                "statements judged",
            ]
            .map(str::to_owned),
        );
    }
    for (i, corpus) in corpora.iter().enumerate() {
        let renders: Vec<&Renders> = records
            .iter()
            .filter(|r| r.corpus == i)
            .filter_map(|r| r.renders.as_ref())
            .collect();
        if renders.is_empty() {
            continue;
        }
        let sum = |f: fn(&Renders) -> usize| renders.iter().map(|r| f(r)).sum::<usize>();
        let mut oracle = VariantVerdicts::default();
        for r in &renders {
            oracle.add(&r.oracle);
        }
        let mut row = vec![
            corpus.name.clone(),
            renders.len().to_string(),
            sum(|r| usize::from(r.sampled)).to_string(),
            sum(|r| r.variants).to_string(),
            sum(|r| r.rendered).to_string(),
            sum(|r| r.variants - r.rendered).to_string(),
            sum(|r| r.differences.len()).to_string(),
        ];
        if with_oracle {
            row.extend(
                [
                    oracle.judged,
                    oracle.accepted,
                    oracle.same,
                    oracle.differs,
                    oracle.statements_judged,
                ]
                .map(|n| n.to_string()),
            );
        }
        rows.push(row);
    }
    if rows.len() == 1 {
        return String::new();
    }
    let mut out =
        String::from("\ntemplates, rendered per combination of branches and loop count\n");
    out.push_str(&table(&rows));
    out
}

/// `part` as a percentage of `whole`, to one decimal.
fn percent(part: usize, whole: usize) -> String {
    if whole == 0 {
        return String::new();
    }
    let tenths = (part * 1000 + whole / 2) / whole;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// The statement kinds the summary lists on their own.
const KINDS: [&str; 16] = [
    "select",
    "with",
    "create table",
    "create materialized view",
    "insert",
    "drop table",
    "set",
    "optimize",
    "alter table",
    "create view",
    "create dictionary",
    "create database",
    "create function",
    "delete",
    "update",
    "create index",
];

/// How much parsed: statements per kind, the expression probe's items, and
/// the most common reasons for a fallback.
fn parse_summary(corpora: &[Corpus], records: &[Record], transform: Transform) -> String {
    let mut rows = vec![
        [
            "corpus",
            "statement kind",
            "statements",
            "parsed",
            "opaque",
            "partial",
            "verbatim",
            "clauses",
            "verbatim clauses",
            "items",
            "verbatim items",
        ]
        .map(str::to_owned)
        .to_vec(),
    ];
    // Parsed whole, with regions kept as written by design, and with a
    // fallback: the share of the first two is how much the grammar covers.
    let mut probe_rows = vec![
        [
            "corpus",
            "probed statements",
            "parsed",
            "opaque",
            "fallback",
            "no fallback %",
            "items",
            "parsed",
            "opaque",
            "fallback",
            "no fallback %",
        ]
        .map(str::to_owned)
        .to_vec(),
    ];
    let mut reasons: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    let percent = |part: usize, all: usize| {
        if all == 0 {
            String::new()
        } else {
            #[expect(clippy::cast_precision_loss, reason = "counts are far below 2^52")]
            let share = part as f64 * 100.0 / all as f64;
            format!("{share:.1}")
        }
    };
    for (i, corpus) in corpora.iter().enumerate() {
        let statements: Vec<&StatementRow> = records
            .iter()
            .filter(|r| r.corpus == i)
            .filter_map(|r| r.parse.as_ref())
            .flat_map(|p| &p.statements)
            .collect();
        let mut groups: Vec<(String, Vec<&StatementRow>)> = KINDS
            .iter()
            .map(|kind| {
                let of = statements
                    .iter()
                    .copied()
                    .filter(|s| s.kind == *kind)
                    .collect();
                ((*kind).to_owned(), of)
            })
            .collect();
        groups.push((
            "other".to_owned(),
            statements
                .iter()
                .copied()
                .filter(|s| !KINDS.contains(&s.kind.as_str()))
                .collect(),
        ));
        groups.push(("all".to_owned(), statements.clone()));
        for (kind, of) in groups {
            if of.is_empty() {
                continue;
            }
            let count = |o: StatementOutcome| of.iter().filter(|s| s.outcome == o).count();
            let sum = |f: &dyn Fn(&StatementRow) -> usize| of.iter().map(|s| f(s)).sum::<usize>();
            rows.push(vec![
                corpus.name.clone(),
                kind,
                of.len().to_string(),
                count(StatementOutcome::Parsed).to_string(),
                count(StatementOutcome::Opaque).to_string(),
                count(StatementOutcome::Partial).to_string(),
                count(StatementOutcome::Verbatim).to_string(),
                sum(&|s| s.clauses).to_string(),
                sum(&|s| s.verbatim_clauses).to_string(),
                sum(&|s| s.items).to_string(),
                sum(&|s| s.verbatim_items).to_string(),
            ]);
        }
        let probed: Vec<(ProbeItems, StatementOutcome)> = statements
            .iter()
            .filter_map(|s| s.probe_items.map(|items| (items, s.outcome)))
            .collect();
        if !probed.is_empty() {
            let count = |o: StatementOutcome| probed.iter().filter(|p| p.1 == o).count();
            let (parsed, opaque) = (
                count(StatementOutcome::Parsed),
                count(StatementOutcome::Opaque),
            );
            let items = probed
                .iter()
                .fold(ProbeItems::default(), |a, (b, _)| ProbeItems {
                    parsed: a.parsed + b.parsed,
                    opaque: a.opaque + b.opaque,
                    fallback: a.fallback + b.fallback,
                });
            probe_rows.push(vec![
                corpus.name.clone(),
                probed.len().to_string(),
                parsed.to_string(),
                opaque.to_string(),
                (probed.len() - parsed - opaque).to_string(),
                percent(parsed + opaque, probed.len()),
                items.all().to_string(),
                items.parsed.to_string(),
                items.opaque.to_string(),
                items.fallback.to_string(),
                percent(items.parsed + items.opaque, items.all()),
            ]);
        }
        for statement in &statements {
            if let Some(reason) = &statement.fallback {
                *reasons.entry(reason.as_str()).or_default() += 1;
            }
        }
    }
    let mut out = String::from("\n");
    out.push_str(&table(&rows));
    if transform == Transform::Format {
        out.push('\n');
        out.push_str(&layout_summary(corpora, records));
    }
    if transform == Transform::Recase(Entry::SelectListProbe) {
        out.push('\n');
        out.push_str(&table(&probe_rows));
    }
    if !reasons.is_empty() {
        let mut reasons: Vec<(&str, usize)> = reasons.into_iter().collect();
        reasons.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let mut rows = vec![vec!["fallbacks".to_owned(), "first reason".to_owned()]];
        for (reason, count) in reasons.into_iter().take(15) {
            rows.push(vec![count.to_string(), reason.to_owned()]);
        }
        out.push('\n');
        out.push_str(&table(&rows));
    }
    out
}

/// The reasons a statement is kept as written, as the layout table's
/// columns.
const KEPT: [Kept; 8] = [
    Kept::NotParsed,
    Kept::NoRule,
    Kept::TagPlace,
    Kept::TagTouching,
    Kept::RenderJoined,
    Kept::TemplateData,
    Kept::HiddenTag,
    Kept::RenderDelimiter,
];

/// What the layout did: statements laid out and kept as written, per kind,
/// with the reasons for keeping them.
fn layout_summary(corpora: &[Corpus], records: &[Record]) -> String {
    let mut header = vec![
        "corpus".to_owned(),
        "statement kind".to_owned(),
        "statements".to_owned(),
        "formatted".to_owned(),
        "formatted %".to_owned(),
        "kept".to_owned(),
    ];
    header.extend(KEPT.map(|k| k.name().to_owned()));
    let mut rows = vec![header];
    for (i, corpus) in corpora.iter().enumerate() {
        let statements: Vec<&StatementRow> = records
            .iter()
            .filter(|r| r.corpus == i)
            .filter_map(|r| r.parse.as_ref())
            .flat_map(|p| &p.statements)
            .filter(|s| s.decision.is_some())
            .collect();
        let mut kinds: Vec<&str> = KINDS.to_vec();
        kinds.extend(["other", "all"]);
        for kind in kinds {
            let of: Vec<&StatementRow> = statements
                .iter()
                .copied()
                .filter(|s| match kind {
                    "all" => true,
                    "other" => !KINDS.contains(&s.kind.as_str()),
                    kind => s.kind == kind,
                })
                .collect();
            if of.is_empty() {
                continue;
            }
            let count = |d: Decision| of.iter().filter(|s| s.decision == Some(d)).count();
            let formatted = count(Decision::Formatted);
            let mut row = vec![
                corpus.name.clone(),
                kind.to_owned(),
                of.len().to_string(),
                formatted.to_string(),
                percent(formatted, of.len()),
                (of.len() - formatted).to_string(),
            ];
            row.extend(KEPT.map(|k| count(Decision::Kept(k)).to_string()));
            rows.push(row);
        }
    }
    table(&rows)
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

/// The CSV columns, in order. `oracle` and `oracle_class` are the verdict on
/// the file as a whole; the `oracle_*` counts after them are statements.
const CSV_COLUMNS: &str = "corpus,path,mode,bytes,tokens,statements,outcome,changed,idempotent,\
                           format_us,oracle,oracle_class,oracle_statements,oracle_judged,\
                           oracle_same,oracle_differs,oracle_output_rejected,\
                           oracle_rejected_unrendered_jinja,oracle_rejected_insert_data,\
                           oracle_rejected_empty_query,oracle_rejected_syntax,\
                           oracle_rejected_other,oracle_rejected_failed,oracle_changed,\
                           oracle_changed_judged,round_trip,parsed_statements,\
                           opaque_statements,partial_statements,verbatim_statements,clauses,\
                           verbatim_clauses,items,verbatim_items,formatted_statements,\
                           kept_statements,detail";

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
        let mut fields = vec![
            quote(&corpora[r.corpus].name),
            quote(&r.path),
            if r.mode == Mode::Jinja {
                "jinja"
            } else {
                "sql"
            }
            .to_owned(),
            r.bytes.to_string(),
            r.tokens.to_string(),
            r.statements.to_string(),
            r.outcome.name().to_owned(),
            flag(r.changed).to_owned(),
            flag(r.idempotent).to_owned(),
            r.format_us.to_string(),
            r.oracle.map_or("", OracleOutcome::name).to_owned(),
            class.to_owned(),
        ];
        let counts = r.oracle_counts.map(|c| {
            let mut counts = vec![
                c.statements,
                c.judged(),
                c.same,
                c.differs,
                c.output_rejected,
            ];
            counts.extend(RejectionClass::ALL.map(|class| c.rejected(class)));
            counts.extend([c.changed, c.changed_judged]);
            counts
        });
        match counts {
            Some(counts) => fields.extend(counts.iter().map(ToString::to_string)),
            None => fields.extend(std::iter::repeat_n(String::new(), 13)),
        }
        let statements = r.parse.as_ref().map_or(&[][..], |p| &p.statements[..]);
        let count = |o: StatementOutcome| statements.iter().filter(|s| s.outcome == o).count();
        let sum = |f: fn(&StatementRow) -> usize| statements.iter().map(f).sum::<usize>();
        fields.push(flag(r.parse.as_ref().map(|p| p.round_trip)).to_owned());
        fields.extend(
            [
                count(StatementOutcome::Parsed),
                count(StatementOutcome::Opaque),
                count(StatementOutcome::Partial),
                count(StatementOutcome::Verbatim),
                sum(|s| s.clauses),
                sum(|s| s.verbatim_clauses),
                sum(|s| s.items),
                sum(|s| s.verbatim_items),
            ]
            .map(|n| n.to_string()),
        );
        if r.idempotent.is_none() {
            fields.extend([String::new(), String::new()]);
        } else {
            let formatted = r
                .layout
                .iter()
                .filter(|d| **d == Decision::Formatted)
                .count();
            fields.extend([formatted, r.layout.len() - formatted].map(|n| n.to_string()));
        }
        fields.push(quote(r.detail.lines().next().unwrap_or("")));
        out.push_str(&fields.join(","));
        out.push('\n');
    }
    out
}

/// One row per statement.
fn statements_csv(corpora: &[Corpus], records: &[Record]) -> String {
    let mut out = String::from(
        "corpus,path,statement,kind,outcome,layout,clauses,verbatim_clauses,items,\
         verbatim_items,opaque,probe_items,probe_opaque_items,probe_fallback_items,fallback,\
         text\n",
    );
    for r in records {
        let Some(parse) = &r.parse else {
            continue;
        };
        for (n, s) in parse.statements.iter().enumerate() {
            let [probe_items, probe_opaque, probe_failed] = s
                .probe_items
                .map_or([String::new(), String::new(), String::new()], |items| {
                    [items.all(), items.opaque, items.fallback].map(|n| n.to_string())
                });
            let _ = writeln!(
                out,
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                quote(&corpora[r.corpus].name),
                quote(&r.path),
                n + 1,
                quote(&s.kind),
                s.outcome.name(),
                s.decision.map_or("", Decision::name),
                s.clauses,
                s.verbatim_clauses,
                s.items,
                s.verbatim_items,
                s.opaque,
                probe_items,
                probe_opaque,
                probe_failed,
                quote(s.fallback.as_deref().unwrap_or("")),
                quote(&s.excerpt),
            );
        }
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

#[cfg(test)]
mod tests {
    use chairlift::oracle::Rejection;
    use chairlift::oracle::StatementReport;

    use super::*;

    #[test]
    fn differing_statements_are_located() {
        let statement = |line, judgement| StatementReport {
            line,
            output_line: line + 1,
            changed: true,
            judgement,
        };
        let differs = Judgement::Differs {
            input: "SELECT a + b FROM t;\n".to_owned(),
            output: "SELECT a - b FROM t;\n".to_owned(),
        };
        let syntax = Judgement::Unjudgeable(Rejection {
            class: RejectionClass::Syntax,
            message: "x".to_owned(),
        });
        let report = Report {
            whole: syntax.clone(),
            statements: vec![statement(1, syntax), statement(4, differs.clone())],
            mismatch: None,
        };
        assert_eq!(
            findings("", &report),
            [
                "statement 2 (line 4, output line 5): the oracle prints \"SELECT a + b FROM t;\" \
                 for the input and \"SELECT a - b FROM t;\" for the output"
            ]
        );
        // A difference only the whole texts show.
        let report = Report {
            whole: differs,
            statements: vec![statement(1, Judgement::Same)],
            mismatch: None,
        };
        let found = findings("variant", &report);
        assert_eq!(found.len(), 1);
        assert!(
            found[0].starts_with("variant: the whole text, though no statement on its own: "),
            "{found:?}"
        );
    }
}
