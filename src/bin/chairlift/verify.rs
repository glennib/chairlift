//! `--verify`: whether ClickHouse reads each formatted file as it reads the
//! file before formatting, as judged by `clickhouse-format` through
//! [`chairlift::oracle`].
//!
//! Only files that formatting changed are judged; the others are their own
//! input. All of them go through one [`Oracle::judge_pairs`] call, which
//! batches texts per process, judges statement by statement where a whole
//! text is rejected, and runs a text alone where a batch fails on it. A file
//! is judged as it is and, built with the `template` feature, a template
//! once per rendering ([`oracle::variants`]); without it a template's
//! statements that hold Jinja are not judged, which [`Verdict::unrendered`]
//! records.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use chairlift::lexer::Mode;
use chairlift::oracle;
use chairlift::oracle::Counts;
use chairlift::oracle::Judgement;
use chairlift::oracle::Oracle;
use chairlift::oracle::Pair;
use chairlift::oracle::RejectionClass;
use chairlift::oracle::Report;
use chairlift::oracle::TIMEOUTS_IN_A_ROW;

use crate::diagnostic;
use crate::diagnostic::Diagnostic;

/// The program `--verify` runs when `--clickhouse-binary` is not given.
const DEFAULT_BINARY: &str = "clickhouse-format";

/// `clickhouse-format`, checked to run.
pub struct Verifier {
    oracle: Oracle,
}

/// A formatted file to judge.
pub struct Job<'a> {
    pub name: &'a str,
    pub input: &'a str,
    pub output: &'a str,
    pub mode: Mode,
}

/// What the oracle made of one file.
#[derive(Debug, Default)]
pub struct Verdict {
    /// Statement counts over every text judged for the file.
    pub counts: Counts,
    /// Set when the oracle reads the output differently from the input: a
    /// bug in chairlift.
    pub bug: Option<Diagnostic>,
    /// Whether the file is a template judged without rendering, so that its
    /// statements holding Jinja were not judged.
    pub unrendered: bool,
    /// A warning that the whole texts read differently although every
    /// statement reads the same.
    pub whole_only: Option<Diagnostic>,
}

impl Verifier {
    /// The oracle run as `program`, or as `clickhouse-format` found on
    /// `PATH`, with up to `jobs` processes at a time; an error if it does
    /// not run or does not behave like `clickhouse-format`.
    pub fn new(
        program: Option<PathBuf>,
        jobs: usize,
        timeout: Option<Duration>,
    ) -> Result<Verifier, Diagnostic> {
        let mut oracle = Oracle::new(program.unwrap_or_else(|| PathBuf::from(DEFAULT_BINARY)));
        oracle.jobs = jobs.max(1);
        if let Some(timeout) = timeout {
            oracle.timeout = timeout;
        }
        // The oracle's processes lead groups of their own, which Ctrl-C does
        // not reach.
        oracle::stop_running_on_signals();
        match oracle.probe() {
            Ok(()) => Ok(Verifier { oracle }),
            Err(e) => Err(Diagnostic::error(
                "chairlift::verify::binary",
                format!("--verify cannot use ClickHouse's formatter: {e}"),
            )
            .with_help(
                "--verify runs `clickhouse-format`, which comes with ClickHouse \
                 (https://clickhouse.com/docs/install): put it on PATH, or pass \
                 --clickhouse-binary with the path to `clickhouse-format` or to the \
                 `clickhouse` binary",
            )),
        }
    }

    /// The diagnostic saying that the oracle stopped running the program,
    /// if it did: then no statement it failed on was really tried.
    pub fn gave_up(&self) -> Option<Diagnostic> {
        self.oracle.gave_up().then(|| {
            Diagnostic::error(
                "chairlift::verify::gave_up",
                format!(
                    "clickhouse-format timed out {TIMEOUTS_IN_A_ROW} times in a row, so --verify \
                     stopped running it"
                ),
            )
        })
    }

    /// The verdict on each job, in order.
    pub fn verify(&self, jobs: &[Job<'_>]) -> Vec<Verdict> {
        let mut owners = Vec::new();
        let mut variants = Vec::new();
        for (j, job) in jobs.iter().enumerate() {
            for variant in oracle::variants(job.input, job.output, job.mode) {
                owners.push(j);
                variants.push(variant);
            }
        }
        let pairs: Vec<Pair<'_>> = variants
            .iter()
            .map(|v| Pair {
                input: &v.input,
                output: &v.output,
                mode: v.mode,
            })
            .collect();
        let reports = self.oracle.judge_pairs(&pairs);
        let mut verdicts: Vec<Verdict> = jobs.iter().map(|_| Verdict::default()).collect();
        let mut rendered = vec![false; jobs.len()];
        for ((j, variant), report) in owners.into_iter().zip(&variants).zip(&reports) {
            let verdict = &mut verdicts[j];
            verdict.counts.add(&report.counts());
            rendered[j] |= variant.rendering.is_some();
            if verdict.bug.is_some() {
                continue;
            }
            if report.mismatch.is_some() || report.statements.iter().any(|s| is_bad(&s.judgement)) {
                verdict.bug = Some(bug(&jobs[j], variant.rendering.as_deref(), report));
            } else if is_bad(&report.whole) && verdict.whole_only.is_none() {
                verdict.whole_only = Some(whole_only(&jobs[j]));
            }
        }
        for ((verdict, job), rendered) in verdicts.iter_mut().zip(jobs).zip(rendered) {
            verdict.unrendered = job.mode == Mode::Jinja
                && !rendered
                && verdict.counts.rejected(RejectionClass::UnrenderedJinja) > 0;
        }
        verdicts
    }
}

/// Whether a judgement shows a formatter bug.
fn is_bad(judgement: &Judgement) -> bool {
    matches!(
        judgement,
        Judgement::Differs { .. } | Judgement::OutputRejected(_)
    )
}

/// The warning for whole texts that read differently although each
/// statement reads the same.
///
/// The statements are what ClickHouse runs, split as `clickhouse-client`
/// splits them, so they decide. `clickhouse-format` splits differently in
/// places: it ends `INSERT … VALUES` data at the end of the line, so a
/// statement after it on the same line becomes part of the data.
///
/// It shows nothing the oracle printed: that is printed with
/// `--show_secrets`, so it can hold passwords, and a warning ends up in
/// logs where nobody needs them.
fn whole_only(job: &Job<'_>) -> Diagnostic {
    Diagnostic::warning(
        "chairlift::verify::whole_text",
        format!(
            "{}: clickhouse-format reads the formatted text differently as a whole, though it reads each \
             statement the same; it splits some texts into statements differently from \
             ClickHouse's client, as after `INSERT … VALUES` data",
            job.name
        ),
    )
}

/// The diagnostic for a report that shows a bug in formatting `job`.
fn bug(job: &Job<'_>, rendering: Option<&str>, report: &Report) -> Diagnostic {
    let statement = report
        .statements
        .iter()
        .enumerate()
        .find(|(_, s)| is_bad(&s.judgement));
    let (place, line, judgement) = match (report.mismatch, statement) {
        (Some(m), _) => (
            format!(
                "the input splits into {} statements and the output into {}",
                m.input, m.output
            ),
            None,
            None,
        ),
        (None, Some((n, s))) => (
            format!(
                "statement {} (line {}, line {} of the output)",
                n + 1,
                s.line,
                s.output_line
            ),
            Some(s.line),
            Some(&s.judgement),
        ),
        (None, None) => ("the whole text".to_owned(), None, Some(&report.whole)),
    };
    let mut help = String::new();
    match judgement {
        Some(Judgement::Differs { input, output }) => {
            let _ = writeln!(help, "{}", oracle::difference(input, output));
        }
        Some(Judgement::OutputRejected(why)) => {
            let _ = writeln!(help, "the oracle rejects the output: {}", why.message);
        }
        _ => {}
    }
    help.push_str(&diagnostic::bug_report_help());
    let name = job.name;
    let rendered = rendering.map_or_else(String::new, |r| format!(", rendered with {r},"));
    let diagnostic = Diagnostic::error(
        "chairlift::verify::clickhouse",
        format!(
            "{name}: clickhouse-format reads the formatted text{rendered} differently at \
             {place}, so the file is left unchanged"
        ),
    )
    .with_help(help);
    match (line, rendering) {
        (Some(line), None) => diagnostic.with_snippet(
            name,
            job.input.to_owned(),
            line_offset(job.input, line),
            "this statement",
        ),
        _ => diagnostic,
    }
}

/// The byte offset where 1-based `line` of `text` starts.
fn line_offset(text: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    text.match_indices('\n')
        .nth(line - 2)
        .map_or(text.len(), |(i, _)| i + 1)
}

/// The line that sums up what the oracle judged over every changed file.
pub fn summary(counts: &Counts, unrendered: usize) -> String {
    let mut out = format!(
        "clickhouse-format reads {} of {} statements in changed files the same",
        counts.same, counts.statements
    );
    let not_judged: Vec<String> = RejectionClass::ALL
        .iter()
        .filter(|&&c| counts.rejected(c) > 0)
        .map(|&c| format!("{} {}", counts.rejected(c), class_name(c)))
        .collect();
    if !not_judged.is_empty() {
        let _ = write!(out, "; not judged: {}", not_judged.join(", "));
    }
    if unrendered > 0 {
        let _ = write!(
            out,
            "; {unrendered} template{} not rendered{}",
            if unrendered == 1 { "" } else { "s" },
            if cfg!(feature = "template") {
                ""
            } else {
                " (chairlift is built without the `template` feature)"
            }
        );
    }
    out
}

/// What statements of a class the oracle cannot judge are, after a count.
fn class_name(class: RejectionClass) -> &'static str {
    match class {
        RejectionClass::UnrenderedJinja => "with Jinja",
        RejectionClass::InsertData => "with INSERT data it does not print",
        RejectionClass::EmptyQuery => "empty",
        RejectionClass::Syntax => "it cannot parse",
        RejectionClass::Other => "it cannot print",
        RejectionClass::Failed => "on which it failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_start_after_newlines() {
        let text = "a\nbc\n\nd";
        assert_eq!(line_offset(text, 1), 0);
        assert_eq!(line_offset(text, 2), 2);
        assert_eq!(line_offset(text, 3), 5);
        assert_eq!(line_offset(text, 4), 6);
        assert_eq!(line_offset(text, 9), text.len());
    }
}
