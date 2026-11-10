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
//! judged as a [`Pair`] of its own, in [`Mode::Sql`].
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

use std::io::Read as _;
use std::io::Write as _;
use std::ops::Range;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crate::lexer::Mode;
use crate::lexer::TokenKind;
use crate::lexer::tokenize;
use crate::split::DataFormat;
use crate::split::split;

mod statements;

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
    /// The input does not parse, so the oracle cannot judge.
    Unjudgeable(Rejection),
}

/// Judges an output by the verdicts on it and on its input.
#[must_use]
pub fn judge(input: &Verdict, output: &Verdict) -> Judgement {
    match (input, output) {
        (Verdict::Rejected(why), _) => Judgement::Unjudgeable(why.clone()),
        (Verdict::Accepted(_), Verdict::Rejected(why)) => Judgement::OutputRejected(why.clone()),
        (Verdict::Accepted(a), Verdict::Accepted(b)) if a == b => Judgement::Same,
        (Verdict::Accepted(a), Verdict::Accepted(b)) => Judgement::Differs {
            input: a.clone(),
            output: b.clone(),
        },
    }
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
        }
    }

    /// Whether the program runs and formats `SELECT 1`.
    #[must_use]
    pub fn is_available(&self) -> bool {
        matches!(self.run("SELECT 1"), Ok(run) if run.success)
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
                .map(|w| w.join().unwrap_or_default())
                .collect()
        });
        for (k, verdict) in results.into_iter().flatten() {
            verdicts[k] = Some(verdict);
        }
        verdicts
            .into_iter()
            .map(|v| {
                v.unwrap_or_else(|| {
                    Verdict::Rejected(Rejection {
                        class: RejectionClass::Failed,
                        message: "the oracle's worker thread panicked".to_owned(),
                    })
                })
            })
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
            Err(message) => Verdict::Rejected(Rejection {
                class: RejectionClass::Failed,
                message,
            }),
        }
    }

    fn run(&self, input: &str) -> Result<Run, String> {
        let mut child = Command::new(&self.program)
            .args(["-n", "--oneline", "--show_secrets"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run {}: {e}", self.program.display()))?;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        let mut stdout = child.stdout.take().ok_or("no stdout")?;
        let mut stderr = child.stderr.take().ok_or("no stderr")?;
        thread::scope(|scope| {
            scope.spawn(move || {
                // A process that exits early closes its stdin; what it did
                // not read does not matter then.
                let _ = stdin.write_all(input.as_bytes());
            });
            let out = scope.spawn(move || {
                let mut buf = Vec::new();
                let _ = stdout.read_to_end(&mut buf);
                buf
            });
            let err = scope.spawn(move || {
                let mut buf = Vec::new();
                let _ = stderr.read_to_end(&mut buf);
                buf
            });
            let deadline = Instant::now() + self.timeout;
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) if Instant::now() >= deadline => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(format!("timed out after {:?}", self.timeout));
                    }
                    Ok(None) => thread::sleep(Duration::from_millis(2)),
                    Err(e) => break Err(e.to_string()),
                }
            };
            let stdout = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
            let stderr = String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned();
            status.map(|status| Run {
                success: status.success(),
                stdout,
                stderr,
            })
        })
    }
}

struct Run {
    success: bool,
    stdout: String,
    stderr: String,
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
