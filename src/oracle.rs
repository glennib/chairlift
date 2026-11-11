//! ClickHouse's own formatter as an oracle: whether formatting kept the
//! meaning of a text, as far as `clickhouse-format` can tell.
//!
//! The oracle is `clickhouse-format -n --oneline --show_secrets`, which parses
//! every statement and prints it back in a canonical form; formatting kept a
//! text's meaning if input and output print the same. `--show_secrets` keeps
//! engine arguments and passwords from printing as `[HIDDEN]`, which would
//! hide differences there.
//!
//! - [`prepare`] works around `clickhouse-format` cutting `INSERT … VALUES`
//!   data at the first newline, by joining the rows onto one line. It must be
//!   applied to input and output alike.
//! - [`Oracle::canonical`] runs many texts through few processes: each run
//!   costs about 70 ms of start-up. Texts are batched, with a boundary
//!   statement after each, and when a batch fails the text that failed is run
//!   alone and the rest of the batch goes on, so one bad statement costs one
//!   extra run. Batches stay small, as the work of one run grows with the
//!   square of its statements: 4,000 copies of `SELECT 1;` take about 2 s,
//!   8,000 about 10 s.
//! - [`judge`] compares the verdicts on an input and its output. Only an
//!   output that the oracle rejects or prints differently, for an input it
//!   accepts, is a formatter bug; an input it rejects cannot be judged, and
//!   [`RejectionClass`] says why.
//! - [`Oracle::judge_pairs`] judges formatted files: whole texts first, and
//!   statement by statement wherever the whole texts do not come out the
//!   same, so that one statement the oracle rejects (as the stateless tests'
//!   deliberate errors are) does not hide the rest of its file. [`Report`]
//!   holds the verdicts and [`Counts`] sums them up.
//!
//! Templates must be rendered before the oracle can judge them; unrendered
//! Jinja is classified as [`RejectionClass::UnrenderedJinja`]. A rendering is
//! judged as a [`Pair`] of its own, in [`Mode::Sql`]; [`variants`] lists the
//! texts to judge for a file.
//!
//! [`Oracle::probe`] checks that the program is `clickhouse-format`, or the
//! `clickhouse` binary, which formats as `clickhouse format`. A run is
//! killed after [`Oracle::timeout`], and keeps a bounded part of what the
//! program prints, so a misbehaving program costs time but not memory.
//!
//! # Blind spots
//!
//! The oracle compares what `clickhouse-format` prints, so a change to
//! something it does not print, or prints the same either way, goes unseen:
//!
//! - comments, which it drops;
//! - the case of the keywords it prints in a fixed case (clause words,
//!   operators, `NULL`, `true`, `false`); identifiers and function, type,
//!   engine and setting names keep their case, so re-casing one of those as
//!   a keyword shows;
//! - the `GRANULARITY` of a `text` index, which it always prints as
//!   `GRANULARITY 100000000` (other index types keep theirs).

use std::borrow::Cow;
use std::fmt;
use std::io;
use std::io::Read;
use std::io::Write as _;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crate::lexer::Mode;
use crate::lexer::TokenKind;
use crate::lexer::tokenize;
use crate::split::DataFormat;
use crate::split::split;

mod process;
mod statements;

pub use process::kill_running;
pub use process::stop_running_on_signals;
pub use statements::Counts;
pub use statements::Mismatch;
pub use statements::Pair;
pub use statements::Report;
pub use statements::StatementReport;

/// `clickhouse-format`, with how to batch work for it.
#[derive(Clone, Debug)]
pub struct Oracle {
    program: PathBuf,
    /// Texts per process, at most, in [`canonical`](Oracle::canonical) and
    /// for the whole texts of [`judge_pairs`](Oracle::judge_pairs).
    pub batch_files: usize,
    /// Statements per process, at most, where
    /// [`judge_pairs`](Oracle::judge_pairs) judges statement by statement.
    pub batch_statements: usize,
    /// Bytes per process, roughly: a batch takes texts until it passes this.
    pub batch_bytes: usize,
    /// How long one process may run before it is killed.
    pub timeout: Duration,
    /// How many processes run at once.
    pub jobs: usize,
    /// Timeouts in a row, shared by clones.
    breaker: Arc<Breaker>,
}

/// Runs that timed out in a row, after which [`Oracle`] gives up.
pub const TIMEOUTS_IN_A_ROW: usize = 3;

/// Counts the runs that time out in a row.
#[derive(Debug, Default)]
struct Breaker {
    timeouts: AtomicUsize,
    tripped: AtomicBool,
}

/// What the oracle made of a text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// It parsed; this is its canonical form.
    Accepted(String),
    /// It did not.
    Rejected(Rejection),
}

/// Why the oracle rejected a text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejection {
    /// What kind of failure it was.
    pub class: RejectionClass,
    /// The first line of `clickhouse-format`'s error message.
    pub message: String,
}

/// Kinds of oracle failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RejectionClass {
    /// A Jinja tag: a failure at `{` or `%`, or a missing substitution name.
    UnrenderedJinja,
    /// `Can't format INSERT query`: inline data in a format other than
    /// `Values`, which `clickhouse-format` does not print.
    InsertData,
    /// `Empty query`, as from a comment followed by `;`.
    EmptyQuery,
    /// Any other syntax error.
    Syntax,
    /// Any other error `clickhouse-format` reports, such as `BAD_ARGUMENTS`
    /// for a statement that parses but cannot be printed.
    Other,
    /// The process could not run, crashed or timed out.
    Failed,
}

impl RejectionClass {
    /// Every class, in declaration order.
    pub const ALL: [RejectionClass; 6] = [
        RejectionClass::UnrenderedJinja,
        RejectionClass::InsertData,
        RejectionClass::EmptyQuery,
        RejectionClass::Syntax,
        RejectionClass::Other,
        RejectionClass::Failed,
    ];

    /// A short name, for tables and CSV.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            RejectionClass::UnrenderedJinja => "unrendered_jinja",
            RejectionClass::InsertData => "insert_data",
            RejectionClass::EmptyQuery => "empty_query",
            RejectionClass::Syntax => "syntax",
            RejectionClass::Other => "other",
            RejectionClass::Failed => "failed",
        }
    }
}

/// What the verdicts on an input and its formatted output say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Judgement {
    /// Both parse to the same canonical form.
    Same,
    /// Both parse, to different canonical forms: a formatter bug.
    Differs {
        /// The input's canonical form.
        input: String,
        /// The output's canonical form.
        output: String,
    },
    /// The input parses and the output does not: a formatter bug.
    OutputRejected(Rejection),
    /// The input does not parse, or the program failed on the output (a
    /// [`RejectionClass::Failed`] run says nothing about the text), so the
    /// oracle cannot judge.
    Unjudgeable(Rejection),
}

/// Judges an output by the verdicts on it and on its input.
#[must_use]
pub fn judge(input: &Verdict, output: &Verdict) -> Judgement {
    match (input, output) {
        (Verdict::Rejected(why), _) => Judgement::Unjudgeable(why.clone()),
        (Verdict::Accepted(_), Verdict::Rejected(why)) if why.class == RejectionClass::Failed => {
            Judgement::Unjudgeable(why.clone())
        }
        (Verdict::Accepted(_), Verdict::Rejected(why)) => Judgement::OutputRejected(why.clone()),
        (Verdict::Accepted(a), Verdict::Accepted(b)) if a == b => Judgement::Same,
        (Verdict::Accepted(a), Verdict::Accepted(b)) => Judgement::Differs {
            input: a.clone(),
            output: b.clone(),
        },
    }
}

/// A text the oracle judges for a formatted file, with the output formatted
/// from it: the file as it is, or one rendering of a template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant<'a> {
    /// Which rendering this is, such as the branches it takes; `None` for
    /// the file as it is.
    pub rendering: Option<String>,
    /// The input text.
    pub input: Cow<'a, str>,
    /// The output text.
    pub output: Cow<'a, str>,
    /// The mode both are split in; a rendering is SQL.
    pub mode: Mode,
}

/// The texts the oracle judges for a file formatted from `input` to
/// `output`, both split in `mode`.
///
/// This is the file as it is. In [`Mode::Jinja`], [`Oracle::judge_pairs`]
/// then finds the statements that hold a tag
/// [`UnrenderedJinja`](RejectionClass::UnrenderedJinja) and judges the
/// others. With the `template` feature, a template also gives a variant per
/// distinct pair of renders of input and output, one per combination of its
/// branches and loop count (see [`crate::template`]), in [`Mode::Sql`]: so
/// every statement is judged. Without it, templates are judged unrendered.
#[must_use]
pub fn variants<'a>(input: &'a str, output: &'a str, mode: Mode) -> Vec<Variant<'a>> {
    let mut out = vec![Variant {
        rendering: None,
        input: Cow::Borrowed(input),
        output: Cow::Borrowed(output),
        mode,
    }];
    #[cfg(feature = "template")]
    if mode == Mode::Jinja {
        out.extend(
            crate::template::rendered_pairs(input, output)
                .into_iter()
                .map(|(label, input, output)| Variant {
                    rendering: Some(format!("render {label}")),
                    input: Cow::Owned(input),
                    output: Cow::Owned(output),
                    mode: Mode::Sql,
                }),
        );
    }
    out
}

/// Characters of a canonical form shown before the point where it differs.
const EXCERPT_BEFORE: usize = 40;
/// Characters of a canonical form shown, at most.
const EXCERPT: usize = 160;

/// Two canonical forms, cut to where they first differ, in a sentence.
#[must_use]
pub fn difference(input: &str, output: &str) -> String {
    let (a, b) = (input.trim_end(), output.trim_end());
    let first = a
        .char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or(a.len().min(b.len()), |((i, _), _)| i);
    // The texts agree up to `first`, so a boundary in one is in both.
    let start = a.floor_char_boundary(first.saturating_sub(EXCERPT_BEFORE));
    let cut = |s: &str| {
        let rest = &s[start..];
        let mut out = String::new();
        if start > 0 {
            out.push('…');
        }
        out.extend(rest.chars().take(EXCERPT));
        if rest.chars().nth(EXCERPT).is_some() {
            out.push('…');
        }
        out
    };
    format!(
        "the oracle prints {:?} for the input and {:?} for the output",
        cut(a),
        cut(b)
    )
}

/// `text` with the rows of every multi-line `INSERT … VALUES` joined onto
/// one line, so that `clickhouse-format`, which takes the data only up to
/// the first newline, sees all of it.
///
/// Inside the data, each run of whitespace that holds a line break becomes a
/// space and each comment is dropped; strings are left alone, so a newline
/// inside a string still cuts the data. `mode` is the mode the text is split
/// in.
#[must_use]
pub fn prepare(text: &str, mode: Mode) -> String {
    let split = split(text, mode);
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for statement in split.statements() {
        let Some(data) = statement.data else {
            continue;
        };
        let token = split.tokens()[data.token];
        let rows = token.text(text);
        if data.format != DataFormat::Values || !rows.contains('\n') {
            continue;
        }
        out.push_str(&text[pos..token.start()]);
        for row_token in tokenize(rows, mode) {
            let piece = row_token.text(rows);
            match row_token.kind() {
                TokenKind::Whitespace | TokenKind::BlockComment if piece.contains('\n') => {
                    out.push(' ');
                }
                TokenKind::LineComment => out.push(' '),
                _ => out.push_str(piece),
            }
        }
        pos = token.end();
    }
    out.push_str(&text[pos..]);
    out
}

impl Oracle {
    /// The oracle run as `program` (such as `clickhouse-format`, found on
    /// `PATH`), with batches of up to 200 texts (50 when they are statements)
    /// or 32 KiB, a 60 s timeout per process, and one process per available
    /// CPU.
    #[must_use]
    pub fn new(program: impl Into<PathBuf>) -> Oracle {
        Oracle {
            program: program.into(),
            batch_files: 200,
            batch_statements: 50,
            batch_bytes: 32 << 10,
            timeout: Duration::from_secs(60),
            jobs: thread::available_parallelism().map_or(1, usize::from),
            breaker: Arc::default(),
        }
    }

    /// Whether the oracle has stopped running the program, because it timed
    /// out [`TIMEOUTS_IN_A_ROW`] times in a row: from then on every text
    /// fails with [`RejectionClass::Failed`] at once.
    #[must_use]
    pub fn gave_up(&self) -> bool {
        self.breaker.tripped.load(Ordering::SeqCst)
    }

    /// Whether the program runs and formats `SELECT 1`.
    #[must_use]
    pub fn is_available(&self) -> bool {
        matches!(self.run("SELECT 1"), Ok(run) if run.success)
    }

    /// Checks that the program runs and formats as `clickhouse-format` does:
    /// that `select 1` comes back as `SELECT 1;`, within [`PROBE_TIMEOUT`]
    /// or the timeout if that is shorter.
    ///
    /// # Errors
    ///
    /// If the program is not found, cannot be run, or does something else.
    pub fn probe(&self) -> Result<(), ProbeError> {
        let program = self.program.display().to_string();
        let run = match self.run_for("select 1", self.timeout.min(PROBE_TIMEOUT)) {
            Ok(run) => run,
            // A name without a `/` is looked up on `PATH`, and a directory
            // fails as if it were not executable.
            Err(RunError::Spawn(_)) if self.program.is_dir() => {
                return Err(ProbeError::Directory { program });
            }
            Err(RunError::Spawn(e)) if e.kind() == io::ErrorKind::NotFound => {
                return Err(ProbeError::NotFound { program });
            }
            Err(e) => {
                return Err(ProbeError::CannotRun {
                    program,
                    message: e.to_string(),
                });
            }
        };
        // Several statements a run (`-n`) print each with its `;`.
        if run.success && run.stdout.trim().trim_end_matches(';') == "SELECT 1" {
            return Ok(());
        }
        let what = if run.success {
            format!("prints {:?}", excerpt(run.stdout.trim()))
        } else {
            let message = classify(&run.stderr).message;
            if message.is_empty() {
                "fails without a message".to_owned()
            } else {
                format!("fails with {:?}", excerpt(&message))
            }
        };
        Err(ProbeError::Unexpected { program, what })
    }

    /// The oracle's verdict on each text, in order.
    #[must_use]
    pub fn canonical(&self, texts: &[&str]) -> Vec<Verdict> {
        self.canonical_in_batches(texts, self.batch_files, Fallback::Alone)
    }

    /// The verdict on each text, in order, with up to `batch_texts` texts to
    /// a process.
    ///
    /// Batches are tasks on a queue that every job takes from, and so is the
    /// work a failed batch leaves: the texts after the one that failed, as a
    /// batch, and with [`Fallback::Alone`] the one that failed, to run alone.
    /// The fallbacks of one batch thus run in parallel.
    fn canonical_in_batches(
        &self,
        texts: &[&str],
        batch_texts: usize,
        fallback: Fallback,
    ) -> Vec<Verdict> {
        let jobs = self.jobs.max(1);
        // A few texts still go to every job.
        let per_batch = batch_texts.min(texts.len().div_ceil(jobs)).max(1);
        let mut tasks = Vec::new();
        let mut start = 0;
        while start < texts.len() {
            let mut end = start;
            let mut bytes = 0;
            while end < texts.len()
                && end - start < per_batch
                && (end == start || bytes < self.batch_bytes)
            {
                bytes += texts[end].len();
                end += 1;
            }
            tasks.push(Task::Batch(start..end));
            start = end;
        }
        // The queue is a stack: the first batch goes first.
        tasks.reverse();
        let queue = Queue {
            state: Mutex::new(QueueState { tasks, running: 0 }),
            changed: Condvar::new(),
        };
        let mut verdicts: Vec<Option<Verdict>> = vec![None; texts.len()];
        let results: Vec<Vec<(usize, Verdict)>> = thread::scope(|scope| {
            let workers: Vec<_> = (0..jobs.min(texts.len()))
                .map(|_| {
                    scope.spawn(|| {
                        let mut done = Vec::new();
                        while let Some(task) = queue.take() {
                            let mut running = Running {
                                queue: &queue,
                                more: Vec::new(),
                            };
                            running.more = self.task(task, texts, fallback, &mut done);
                        }
                        done
                    })
                })
                .collect();
            workers
                .into_iter()
                // A panic is a bug here, not a verdict on a text: it must not
                // pass as "not judged".
                .map(|w| {
                    w.join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                })
                .collect()
        });
        for (k, verdict) in results.into_iter().flatten() {
            verdicts[k] = Some(verdict);
        }
        verdicts
            .into_iter()
            .map(|v| v.expect("every task leaves a verdict or more tasks"))
            .collect()
    }

    /// Runs `task` on `texts`, pushing the verdicts it reaches onto `done`,
    /// and returns the tasks it leaves.
    fn task(
        &self,
        task: Task,
        texts: &[&str],
        fallback: Fallback,
        done: &mut Vec<(usize, Verdict)>,
    ) -> Vec<Task> {
        let range = match task {
            Task::Alone(k) => {
                done.push((k, self.alone(texts[k])));
                return Vec::new();
            }
            Task::Batch(range) if range.len() == 1 => {
                done.push((range.start, self.alone(texts[range.start])));
                return Vec::new();
            }
            Task::Batch(range) => range,
        };
        let input: String = range
            .clone()
            .enumerate()
            .map(|(n, k)| [texts[k], &*terminator(texts[k]), &boundary(n)].concat())
            .collect();
        let (outputs, stderr) = match self.run(&input) {
            Ok(run) => (split_batch_output(&run.stdout), run.stderr),
            Err(_) => (Vec::new(), String::new()),
        };
        let completed = outputs.len().min(range.len());
        for (k, output) in range.clone().zip(outputs) {
            done.push((k, Verdict::Accepted(output)));
        }
        let failed = range.start + completed;
        if failed == range.end {
            return Vec::new();
        }
        let mut more = Vec::new();
        if failed + 1 < range.end {
            more.push(Task::Batch(failed + 1..range.end));
        }
        // The text after the last boundary printed failed.
        let rejection = classify(&stderr);
        if fallback == Fallback::BatchError && rejection.class != RejectionClass::Failed {
            done.push((failed, Verdict::Rejected(rejection)));
        } else {
            more.push(Task::Alone(failed));
        }
        more
    }

    /// The verdict on one text, run on its own.
    fn alone(&self, text: &str) -> Verdict {
        match self.run(text) {
            Ok(run) if run.success => Verdict::Accepted(normalize(&run.stdout)),
            Ok(run) => Verdict::Rejected(classify(&run.stderr)),
            Err(e) => Verdict::Rejected(Rejection {
                class: RejectionClass::Failed,
                message: e.to_string(),
            }),
        }
    }

    /// Counts a run that ended, by timing out or otherwise.
    fn ran(&self, timed_out: bool) {
        let breaker = &self.breaker;
        if !timed_out {
            breaker.timeouts.store(0, Ordering::SeqCst);
        } else if breaker.timeouts.fetch_add(1, Ordering::SeqCst) + 1 >= TIMEOUTS_IN_A_ROW {
            breaker.tripped.store(true, Ordering::SeqCst);
        }
    }

    /// Runs the program on `input`, with the timeout.
    fn run(&self, input: &str) -> Result<Run, RunError> {
        self.run_for(input, self.timeout)
    }

    /// Runs the program on `input`.
    ///
    /// The program runs in a process group of its own, which is killed when
    /// the program runs past `timeout`, so that what it started dies with
    /// it. What it prints is kept up to a limit, a multiple of the input's
    /// size for stdout; more stdout than that kills it and fails the run, at
    /// once, and more stderr is read and dropped.
    ///
    /// After [`TIMEOUTS_IN_A_ROW`] runs in a row time out, the program is
    /// not run any more ([`Oracle::gave_up`]): one that hangs on every input
    /// would otherwise cost a timeout per text, and then again per text the
    /// fallbacks split off.
    ///
    /// The pipes are fed and read on threads of their own that this does
    /// not wait for past the timeout: a process that left the group can
    /// hold them open after the program is killed, and the threads end only
    /// when it exits.
    fn run_for(&self, input: &str, timeout: Duration) -> Result<Run, RunError> {
        if self.gave_up() {
            return Err(RunError::Other(format!(
                "not run, as it timed out {TIMEOUTS_IN_A_ROW} times in a row"
            )));
        }
        let mut command = Command::new(&self.program);
        if is_multi_call(&self.program) {
            command.arg("format");
        }
        command
            .args(["-n", "--oneline", "--show_secrets"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        process::own_group(&mut command);
        let mut child = command.spawn().map_err(RunError::Spawn)?;
        let group = process::Group::of(&child);
        let pipes = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let (Some(mut stdin), Some(mut stdout), Some(mut stderr)) = pipes else {
            group.kill(&mut child);
            let _ = child.wait();
            return Err(RunError::Other("the process has no pipes".to_owned()));
        };
        let stdout_limit = input.len().saturating_mul(4).saturating_add(1 << 20);
        let input = input.as_bytes().to_vec();
        thread::spawn(move || {
            // A process that exits early closes its stdin; what it did not
            // read does not matter then.
            let _ = stdin.write_all(&input);
        });
        let (out_sender, out) = mpsc::channel();
        let (err_sender, err) = mpsc::channel();
        thread::spawn(move || out_sender.send(read_limited(&mut stdout, stdout_limit, false)));
        thread::spawn(move || err_sender.send(read_limited(&mut stderr, STDERR_LIMIT, true)));
        let too_much = || RunError::Other(format!("printed more than {stdout_limit} bytes"));
        let deadline = Instant::now() + timeout;
        let mut printed = None;
        let status = loop {
            if printed.is_none()
                && let Ok(read) = out.try_recv()
            {
                if read.1 {
                    group.kill(&mut child);
                    let _ = child.wait();
                    self.ran(false);
                    return Err(too_much());
                }
                printed = Some(read);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    group.kill(&mut child);
                    let _ = child.wait();
                    self.ran(true);
                    return Err(RunError::Other(format!("timed out after {timeout:?}")));
                }
                Ok(None) => thread::sleep(Duration::from_millis(2)),
                Err(e) => {
                    group.kill(&mut child);
                    let _ = child.wait();
                    return Err(RunError::Other(e.to_string()));
                }
            }
        };
        self.ran(false);
        // The program has exited; what is left in the pipes takes moments
        // to read, unless something else holds them open.
        let drained = Instant::now() + PIPE_GRACE;
        let mut receive = |from: &mpsc::Receiver<(Vec<u8>, bool)>| {
            from.recv_timeout(drained.saturating_duration_since(Instant::now()))
                .map_err(|_| {
                    // Unless it left, what holds them is of the group, whose
                    // id is then still taken and cannot name another group.
                    group.kill(&mut child);
                    RunError::Other(
                        "exited, but a process it started holds its output open".to_owned(),
                    )
                })
        };
        let (stdout, stdout_cut) = match printed {
            Some(read) => read,
            None => receive(&out)?,
        };
        let (stderr, _) = receive(&err)?;
        if stdout_cut {
            return Err(too_much());
        }
        Ok(Run {
            success: status.success(),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }
}

/// How long [`Oracle::probe`] waits for `select 1`, so that a program that
/// hangs is reported soon. The first start of the ClickHouse binary from a
/// cold disk can take seconds.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// Bytes of stderr kept from a run: the first lines say what failed.
const STDERR_LIMIT: usize = 64 << 10;

/// How long the pipes are read after the program exits.
const PIPE_GRACE: Duration = Duration::from_secs(5);

/// Reads the first `limit` bytes of `reader`, and whether there are more:
/// with `drain`, reading on to the end and dropping the rest, and without,
/// stopping at the first byte past the limit.
fn read_limited(reader: &mut impl Read, limit: usize, drain: bool) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let _ = reader
        .by_ref()
        .take(u64::try_from(limit).unwrap_or(u64::MAX))
        .read_to_end(&mut kept);
    let more = if drain {
        io::copy(reader, &mut io::sink()).is_ok_and(|n| n > 0)
    } else {
        reader.read(&mut [0]).is_ok_and(|n| n > 0)
    };
    (kept, more)
}

/// Whether `program` is ClickHouse's single binary, `clickhouse`, which
/// formats as `clickhouse format`.
fn is_multi_call(program: &Path) -> bool {
    program
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("clickhouse"))
}

/// The first characters of `text`, for a message.
fn excerpt(text: &str) -> String {
    const SHOWN: usize = 80;
    let mut out: String = text.chars().take(SHOWN).collect();
    if text.chars().nth(SHOWN).is_some() {
        out.push('…');
    }
    out
}

struct Run {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Why a run of the program gave no verdict.
#[derive(Debug)]
enum RunError {
    /// It could not be started.
    Spawn(io::Error),
    /// It timed out, printed too much, or could not be waited for.
    Other(String),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Spawn(e) => write!(f, "cannot be started: {e}"),
            RunError::Other(message) => f.write_str(message),
        }
    }
}

/// Why [`Oracle::probe`] found the program unusable.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ProbeError {
    /// There is no such program.
    #[error("`{program}` was not found")]
    NotFound {
        /// The program, as given.
        program: String,
    },
    /// It is a directory.
    #[error("`{program}` is a directory, not an executable file")]
    Directory {
        /// The program, as given.
        program: String,
    },
    /// It could not be started, or did not finish.
    #[error("`{program}` {message}")]
    CannotRun {
        /// The program, as given.
        program: String,
        /// What went wrong.
        message: String,
    },
    /// It ran, and did not format `select 1` as `SELECT 1`.
    #[error("`{program}` does not behave like clickhouse-format: given `select 1`, it {what}")]
    Unexpected {
        /// The program, as given.
        program: String,
        /// What it did instead.
        what: String,
    },
}

/// What becomes of a text that fails in a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fallback {
    /// It is run again on its own, since it may have failed only because of
    /// what the batch put after it. Each failure costs a process.
    Alone,
    /// It is rejected with the batch's error, unless the process failed
    /// outright. A batch rejects a text that passes alone only where the
    /// text runs into what follows it, which [`terminator`] is there to
    /// prevent; callers re-run alone the rejections a judgement turns on.
    BatchError,
}

/// Work for one `clickhouse-format` process, as indices into the texts.
enum Task {
    Batch(Range<usize>),
    Alone(usize),
}

/// The tasks left, shared by the jobs of [`Oracle::canonical_in_batches`].
struct Queue {
    state: Mutex<QueueState>,
    /// Signalled when a task finishes.
    changed: Condvar,
}

struct QueueState {
    tasks: Vec<Task>,
    /// Tasks taken and not yet finished, which may leave more.
    running: usize,
}

impl Queue {
    /// The next task, waiting while none is left but some may still come;
    /// `None` once all are done.
    fn take(&self) -> Option<Task> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(task) = state.tasks.pop() {
                state.running += 1;
                return Some(task);
            }
            if state.running == 0 {
                return None;
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// A taken task: when dropped, also when its job panics, it is finished and
/// the tasks it left, `more`, are queued.
struct Running<'q> {
    queue: &'q Queue,
    more: Vec<Task>,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        let mut state = self
            .queue
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.running -= 1;
        state.tasks.append(&mut self.more);
        drop(state);
        self.queue.changed.notify_all();
    }
}

/// What ends a text in a batch so that the boundary after it is a statement
/// of its own: a `;` for a last statement without one, on the same line
/// (`clickhouse-format` takes `VALUES` data up to the end of the line) unless
/// the text ends in a line comment, and a newline.
fn terminator(text: &str) -> String {
    let split = split(text, Mode::Sql);
    let unterminated = split
        .statements()
        .last()
        .is_some_and(|s| s.terminator.is_none());
    let ends_in_comment = split
        .tokens()
        .iter()
        .rev()
        .find(|t| t.kind() != TokenKind::Whitespace)
        .is_some_and(|t| t.kind() == TokenKind::LineComment);
    let mut end = String::new();
    if unterminated {
        if ends_in_comment {
            end.push('\n');
        }
        end.push(';');
    }
    end.push('\n');
    end
}

const BOUNDARY: &str = "chairlift-oracle-boundary";

/// The statement that follows the `n`th text of a batch.
fn boundary(n: usize) -> String {
    format!("SELECT '{BOUNDARY}-{n}';\n\n")
}

/// The canonical forms of the texts of a batch, from its output: what comes
/// before each boundary statement, for as many boundaries as were printed.
fn split_batch_output(stdout: &str) -> Vec<String> {
    let mut outputs = Vec::new();
    let mut current = String::new();
    for line in stdout.lines() {
        if line.starts_with(&format!("SELECT '{BOUNDARY}-")) {
            outputs.push(normalize(&current));
            current.clear();
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    outputs
}

/// The canonical form without blank lines, which only separate statements.
fn normalize(stdout: &str) -> String {
    let mut out = String::with_capacity(stdout.len());
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The class and first line of an error message on stderr.
fn classify(stderr: &str) -> Rejection {
    let line = stderr
        .lines()
        .find(|l| l.contains("Exception") || l.contains("Code:"))
        .or_else(|| stderr.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("");
    let message = line
        .split(", Stack trace")
        .next()
        .unwrap_or(line)
        .trim()
        .to_owned();
    let class = if message.contains("substitution name")
        || message.contains("({)")
        || message.contains("(%)")
    {
        RejectionClass::UnrenderedJinja
    } else if message.contains("Can't format INSERT query") {
        RejectionClass::InsertData
    } else if message.contains("Empty query") {
        RejectionClass::EmptyQuery
    } else if message.contains("SYNTAX_ERROR") || message.contains("Syntax error") {
        RejectionClass::Syntax
    } else if message.contains("DB::Exception") {
        RejectionClass::Other
    } else {
        RejectionClass::Failed
    };
    Rejection { class, message }
}

#[cfg(test)]
mod tests;
