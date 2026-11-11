//! Judging formatted outputs against their inputs: as whole texts, and
//! statement by statement wherever a whole text does not come out the same.

use crate::lexer::Mode;
use crate::split::Split;
use crate::split::Statement;
use crate::split::split;

use super::Fallback;
use super::Judgement;
use super::Oracle;
use super::Rejection;
use super::RejectionClass;
use super::Verdict;
use super::judge;
use super::prepare;

/// An input and the output formatted from it, for [`Oracle::judge_pairs`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair<'a> {
    /// The text that was formatted.
    pub input: &'a str,
    /// What formatting it gave.
    pub output: &'a str,
    /// The mode both are split in.
    pub mode: Mode,
}

/// What the oracle made of one [`Pair`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The judgement on the whole texts.
    pub whole: Judgement,
    /// One entry per statement of the input, in order, each judged against
    /// the output's statement at the same position. When the whole texts
    /// are the same, every statement is too. Empty when the statements do
    /// not correspond.
    pub statements: Vec<StatementReport>,
    /// Set when input and output split into different numbers of
    /// statements. Formatting never adds or removes a token, so this is a
    /// formatter or splitter bug.
    pub mismatch: Option<Mismatch>,
}

/// The judgement on one statement of a [`Pair`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatementReport {
    /// The 1-based line of the input that the statement starts on.
    pub line: usize,
    /// The same for the output.
    pub output_line: usize,
    /// Whether the statement's text differs between input and output.
    pub changed: bool,
    /// The judgement on the statement alone. A statement that is only a `;`
    /// is [`EmptyQuery`](RejectionClass::EmptyQuery) without running the
    /// oracle.
    pub judgement: Judgement,
}

/// How many statements input and output split into, when they differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mismatch {
    /// Statements in the input.
    pub input: usize,
    /// Statements in the output.
    pub output: usize,
}

/// Statement counts from one or more [`Report`]s; [`add`](Counts::add) sums
/// them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Statements in the inputs.
    pub statements: usize,
    /// Statements whose text the formatter changed.
    pub changed: usize,
    /// Statements judged the same.
    pub same: usize,
    /// Statements whose canonical forms differ.
    pub differs: usize,
    /// Statements whose input the oracle accepts and whose output it rejects.
    pub output_rejected: usize,
    /// Changed statements among those judged.
    pub changed_judged: usize,
    /// Statements the oracle cannot judge, because it rejects their input or
    /// failed to run on their output, indexed by the position of the class
    /// in [`RejectionClass::ALL`].
    rejected: [usize; RejectionClass::ALL.len()],
}

impl Counts {
    /// Statements the oracle could judge: the same, differing, or with a
    /// rejected output.
    #[must_use]
    pub const fn judged(&self) -> usize {
        self.same + self.differs + self.output_rejected
    }

    /// Statements the oracle cannot judge for `class`.
    #[must_use]
    pub const fn rejected(&self, class: RejectionClass) -> usize {
        self.rejected[class as usize]
    }

    /// Statements the oracle cannot judge, for any reason.
    #[must_use]
    pub fn rejected_total(&self) -> usize {
        self.rejected.iter().sum()
    }

    /// Adds `other` to these counts.
    pub fn add(&mut self, other: &Counts) {
        self.statements += other.statements;
        self.changed += other.changed;
        self.same += other.same;
        self.differs += other.differs;
        self.output_rejected += other.output_rejected;
        self.changed_judged += other.changed_judged;
        for (a, b) in self.rejected.iter_mut().zip(other.rejected) {
            *a += b;
        }
    }
}

impl Report {
    /// The statement counts of this report. A statement mismatch counts the
    /// input's statements and nothing else.
    #[must_use]
    pub fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        if let Some(mismatch) = self.mismatch {
            counts.statements = mismatch.input;
            return counts;
        }
        counts.statements = self.statements.len();
        for statement in &self.statements {
            counts.changed += usize::from(statement.changed);
            match &statement.judgement {
                Judgement::Same => counts.same += 1,
                Judgement::Differs { .. } => counts.differs += 1,
                Judgement::OutputRejected(_) => counts.output_rejected += 1,
                Judgement::Unjudgeable(why) => {
                    counts.rejected[why.class as usize] += 1;
                    continue;
                }
            }
            counts.changed_judged += usize::from(statement.changed);
        }
        counts
    }

    /// Whether the report shows a formatter bug: a statement or the whole
    /// text that differs or has its output rejected, or statements that do
    /// not correspond.
    #[must_use]
    pub fn is_bug(&self) -> bool {
        let bad =
            |j: &Judgement| matches!(j, Judgement::Differs { .. } | Judgement::OutputRejected(_));
        self.mismatch.is_some()
            || bad(&self.whole)
            || self.statements.iter().any(|s| bad(&s.judgement))
    }
}

/// Where the verdict on one side of a statement comes from.
#[derive(Clone, Copy)]
enum Source {
    /// The verdict on this text of the statement batch.
    Text(usize),
    /// An empty statement, which the oracle rejects as `Empty query`.
    Empty,
}

struct Planned {
    line: usize,
    output_line: usize,
    changed: bool,
    /// `None` when the whole texts are the same.
    sources: Option<(Source, Source)>,
}

impl Oracle {
    /// Judges each output against its input.
    ///
    /// The whole texts go through the oracle first, with
    /// [`prepare`] applied. Where they do not come out the
    /// same (one of them is rejected, or they differ), both are split with
    /// chairlift's statement splitter and each statement is judged against
    /// the output's statement at the same position, so that a statement the
    /// oracle rejects hides only itself. A statement whose text did not change
    /// is run once, and up to [`batch_statements`](Oracle::batch_statements)
    /// statements go to a process. A text that fails in a batch takes the
    /// batch's error, and is run again alone only where its judgement turns
    /// on that: where the other text of its pair is accepted.
    ///
    /// In [`Mode::Jinja`], statements that hold a tag are rejected as
    /// [`UnrenderedJinja`](RejectionClass::UnrenderedJinja); the others are
    /// judged as they are. Tags between statements are not part of either.
    #[must_use]
    pub fn judge_pairs(&self, pairs: &[Pair<'_>]) -> Vec<Report> {
        let mut texts = Vec::new();
        let mut slots = Vec::with_capacity(pairs.len());
        for pair in pairs {
            let input = texts.len();
            texts.push(prepare(pair.input, pair.mode));
            let output = if pair.output == pair.input {
                input
            } else {
                texts.push(prepare(pair.output, pair.mode));
                texts.len() - 1
            };
            slots.push((input, output));
        }
        let verdicts = self.verdicts(&texts, self.batch_files, slots.iter().copied());
        let wholes: Vec<Judgement> = slots
            .iter()
            .map(|&(input, output)| judge(&verdicts[input], &verdicts[output]))
            .collect();
        drop(verdicts);

        let mut texts = Vec::new();
        let mut plans = Vec::with_capacity(pairs.len());
        for (pair, whole) in pairs.iter().zip(&wholes) {
            plans.push(plan(pair, whole == &Judgement::Same, &mut texts));
        }
        let compared = plans
            .iter()
            .filter_map(|plan| plan.as_ref().ok())
            .flatten()
            .filter_map(|p| match p.sources {
                Some((Source::Text(a), Source::Text(b))) => Some((a, b)),
                _ => None,
            });
        let verdicts = self.verdicts(&texts, self.batch_statements, compared);
        let verdict = |source: Source| match source {
            Source::Text(i) => verdicts[i].clone(),
            Source::Empty => Verdict::Rejected(Rejection {
                class: RejectionClass::EmptyQuery,
                message: "an empty statement".to_owned(),
            }),
        };
        wholes
            .into_iter()
            .zip(plans)
            .map(|(whole, plan)| match plan {
                Err(mismatch) => Report {
                    whole,
                    statements: Vec::new(),
                    mismatch: Some(mismatch),
                },
                Ok(planned) => Report {
                    whole,
                    statements: planned
                        .into_iter()
                        .map(|p| StatementReport {
                            line: p.line,
                            output_line: p.output_line,
                            changed: p.changed,
                            judgement: p.sources.map_or(Judgement::Same, |(input, output)| {
                                judge(&verdict(input), &verdict(output))
                            }),
                        })
                        .collect(),
                    mismatch: None,
                },
            })
            .collect()
    }

    /// The verdicts on `texts`, which `compared` pairs up as input and
    /// output.
    ///
    /// A text that fails in a batch takes the batch's error, which saves a
    /// process per rejected statement, a cost that dominates the run where
    /// a corpus holds thousands of deliberate errors. Where a judgement turns
    /// on such a rejection, because the other text of its pair is accepted,
    /// the rejected text is run again alone.
    fn verdicts(
        &self,
        texts: &[String],
        batch_texts: usize,
        compared: impl Iterator<Item = (usize, usize)>,
    ) -> Vec<Verdict> {
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let mut verdicts = self.canonical_in_batches(&refs, batch_texts, Fallback::BatchError);
        let mut decisive: Vec<usize> = compared
            .filter_map(|(a, b)| match (&verdicts[a], &verdicts[b]) {
                (Verdict::Rejected(_), Verdict::Accepted(_)) => Some(a),
                (Verdict::Accepted(_), Verdict::Rejected(_)) => Some(b),
                _ => None,
            })
            .collect();
        decisive.sort_unstable();
        decisive.dedup();
        let again: Vec<&str> = decisive.iter().map(|&k| refs[k]).collect();
        let again = self.canonical_in_batches(&again, 1, Fallback::Alone);
        for (k, verdict) in decisive.into_iter().zip(again) {
            verdicts[k] = verdict;
        }
        verdicts
    }
}

/// The statements of `pair`, with the prepared texts the oracle must run for
/// them pushed onto `texts` unless the whole texts came out the same.
fn plan(
    pair: &Pair<'_>,
    whole_same: bool,
    texts: &mut Vec<String>,
) -> Result<Vec<Planned>, Mismatch> {
    let input = split(pair.input, pair.mode);
    let output = split(pair.output, pair.mode);
    if input.statements().len() != output.statements().len() {
        return Err(Mismatch {
            input: input.statements().len(),
            output: output.statements().len(),
        });
    }
    let mut input_lines = Lines::new(pair.input, &input);
    let mut output_lines = Lines::new(pair.output, &output);
    let mut planned = Vec::with_capacity(input.statements().len());
    for (a, b) in input.statements().iter().zip(output.statements()) {
        let a_text = input.text(a, pair.input);
        let b_text = output.text(b, pair.output);
        let changed = a_text != b_text;
        let sources = (!whole_same).then(|| {
            let mut source = |empty: bool, text: &str| {
                if empty {
                    Source::Empty
                } else {
                    texts.push(prepare(text, pair.mode));
                    Source::Text(texts.len() - 1)
                }
            };
            let a_source = source(a.is_empty(), a_text);
            let b_source = if changed {
                source(b.is_empty(), b_text)
            } else {
                a_source
            };
            (a_source, b_source)
        });
        planned.push(Planned {
            line: input_lines.at(a),
            output_line: output_lines.at(b),
            changed,
            sources,
        });
    }
    Ok(planned)
}

/// Line numbers of statements, counted incrementally through a text.
struct Lines<'a> {
    text: &'a str,
    split: &'a Split,
    offset: usize,
    line: usize,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str, split: &'a Split) -> Self {
        Lines {
            text,
            split,
            offset: 0,
            line: 1,
        }
    }

    /// The 1-based line `statement` starts on; statements come in order.
    fn at(&mut self, statement: &Statement) -> usize {
        let start = self.split.tokens()[statement.tokens.start].start();
        self.line += self.text[self.offset..start].matches('\n').count();
        self.offset = start;
        self.line
    }
}
