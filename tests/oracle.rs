//! The `clickhouse-format` oracle, run for real: batching, the per-text and
//! per-statement fallbacks and the classification of failures.
//!
//! `CHAIRLIFT_CLICKHOUSE_FORMAT` names the binary; the default is
//! `clickhouse-format` on `PATH`, which `mise` provides. Without it the tests
//! skip, unless `CHAIRLIFT_REQUIRE_CORPUS` is set.

use chairlift::lexer::Mode;
use chairlift::oracle::Judgement;
use chairlift::oracle::Mismatch;
use chairlift::oracle::Oracle;
use chairlift::oracle::Pair;
use chairlift::oracle::RejectionClass;
use chairlift::oracle::Report;
use chairlift::oracle::Verdict;
use chairlift::oracle::judge;
use chairlift::oracle::prepare;

fn oracle() -> Option<Oracle> {
    let program = std::env::var_os("CHAIRLIFT_CLICKHOUSE_FORMAT")
        .unwrap_or_else(|| "clickhouse-format".into());
    let oracle = Oracle::new(program);
    if oracle.is_available() {
        Some(oracle)
    } else {
        assert!(
            std::env::var_os("CHAIRLIFT_REQUIRE_CORPUS").is_none_or(|v| v.is_empty()),
            "clickhouse-format is unavailable, and CHAIRLIFT_REQUIRE_CORPUS is set"
        );
        eprintln!("skipping: clickhouse-format is unavailable");
        None
    }
}

fn class(verdict: &Verdict) -> Option<RejectionClass> {
    match verdict {
        Verdict::Accepted(_) => None,
        Verdict::Rejected(why) => Some(why.class),
    }
}

#[test]
fn failures_in_a_batch_cost_only_their_own_text() {
    let Some(mut oracle) = oracle() else {
        return;
    };
    let texts = [
        "select 1",
        "select (",
        "insert into t format CSV\n1,2\n",
        "select {{ x }}",
        "-- c\n;",
        "select 2; select 3 -- c",
        "INSERT INTO t VALUES (1),\n(2);",
        "{% if a %}select 4{% endif %}",
        "create table t (a String) engine = ReplicatedMergeTree('/p', '{replica}') order by a",
    ];
    for batch_files in [1, 3, 100] {
        oracle.batch_files = batch_files;
        oracle.jobs = 2;
        let verdicts = oracle.canonical(&texts);
        let classes: Vec<_> = verdicts.iter().map(class).collect();
        assert_eq!(
            classes,
            [
                None,
                Some(RejectionClass::Syntax),
                Some(RejectionClass::InsertData),
                Some(RejectionClass::UnrenderedJinja),
                Some(RejectionClass::EmptyQuery),
                None,
                Some(RejectionClass::Syntax),
                Some(RejectionClass::UnrenderedJinja),
                None,
            ],
            "batches of {batch_files}: {verdicts:#?}"
        );
        assert_eq!(verdicts[0], Verdict::Accepted("SELECT 1;\n".to_owned()));
        assert_eq!(
            verdicts[5],
            Verdict::Accepted("SELECT 2;\nSELECT 3;\n".to_owned())
        );
        // `--show_secrets` keeps engine arguments.
        let Verdict::Accepted(create) = &verdicts[8] else {
            unreachable!()
        };
        assert!(create.contains("'/p', '{replica}'"), "{create}");
    }
}

#[test]
fn joined_values_rows_can_be_judged() {
    let Some(oracle) = oracle() else {
        return;
    };
    let input = "INSERT INTO t VALUES (1),\n  (2);\nSELECT 1;";
    let output = "INSERT INTO t VALUES (1),\n  (2);\n\nSELECT 1;\n";
    let verdicts = oracle.canonical(&[&prepare(input, Mode::Sql), &prepare(output, Mode::Sql)]);
    assert!(matches!(verdicts[0], Verdict::Accepted(_)), "{verdicts:?}");
    assert_eq!(judge(&verdicts[0], &verdicts[1]), Judgement::Same);

    let changed = oracle.canonical(&["SELECT 1 + 2", "SELECT 1 - 2"]);
    assert!(matches!(
        judge(&changed[0], &changed[1]),
        Judgement::Differs { .. }
    ));
}

/// A file with a statement of each kind the oracle rejects, as the stateless
/// tests' files with deliberate errors have, and a statement on lines 4–5
/// that the outputs below alter.
const MIXED: &str = "SELECT 1;\n\
                     SELECT (; -- { clientError SYNTAX_ERROR }\n\
                     \n\
                     SELECT a + b\n\
                     FROM t;\n\
                     -- an empty statement\n\
                     ;\n\
                     INSERT INTO t FORMAT CSV\n\
                     1,2\n\
                     \n\
                     SELECT {{ x }};\n\
                     INSERT INTO t VALUES (1),\n\
                     (2);\n\
                     SELECT 3";

fn statement_classes(report: &Report) -> Vec<Result<(), RejectionClass>> {
    report
        .statements
        .iter()
        .map(|s| match &s.judgement {
            Judgement::Unjudgeable(why) => Err(why.class),
            _ => Ok(()),
        })
        .collect()
}

#[test]
fn a_changed_statement_is_judged_next_to_rejected_ones() {
    let Some(mut oracle) = oracle() else {
        return;
    };
    let altered = MIXED.replace("a + b", "a - b");
    let unparseable = MIXED.replace("a + b", "a + b)");
    let reformatted = MIXED.replace("SELECT a + b\nFROM t;", "SELECT a +   b FROM t;");
    let pairs = [&altered, &unparseable, &reformatted, MIXED].map(|output| Pair {
        input: MIXED,
        output,
        mode: Mode::Sql,
    });
    for batch_statements in [1, 200] {
        oracle.batch_statements = batch_statements;
        let reports = oracle.judge_pairs(&pairs);
        for report in &reports {
            assert!(
                matches!(&report.whole, Judgement::Unjudgeable(why) if why.class == RejectionClass::Syntax),
                "{report:#?}"
            );
            assert_eq!(
                statement_classes(report),
                [
                    Ok(()),
                    Err(RejectionClass::Syntax),
                    Ok(()),
                    Err(RejectionClass::EmptyQuery),
                    Err(RejectionClass::InsertData),
                    Err(RejectionClass::UnrenderedJinja),
                    Ok(()),
                    Ok(()),
                ],
                "{report:#?}"
            );
            let lines: Vec<usize> = report.statements.iter().map(|s| s.line).collect();
            assert_eq!(lines, [1, 2, 4, 7, 8, 11, 12, 14]);
        }
        let [altered, unparseable, reformatted, unchanged] = &reports[..] else {
            unreachable!()
        };

        assert!(altered.is_bug());
        assert_eq!(
            altered.statements[2].judgement,
            Judgement::Differs {
                input: "SELECT a + b FROM t;\n".to_owned(),
                output: "SELECT a - b FROM t;\n".to_owned(),
            }
        );
        let counts = altered.counts();
        assert_eq!(
            (counts.statements, counts.changed, counts.changed_judged),
            (8, 1, 1)
        );
        assert_eq!(
            (counts.same, counts.differs, counts.output_rejected),
            (3, 1, 0)
        );
        assert_eq!(counts.rejected_total(), 4);
        assert_eq!(counts.rejected(RejectionClass::Syntax), 1);

        assert!(unparseable.is_bug());
        assert!(matches!(
            &unparseable.statements[2].judgement,
            Judgement::OutputRejected(why) if why.class == RejectionClass::Syntax
        ));
        assert_eq!(unparseable.counts().output_rejected, 1);

        assert!(!reformatted.is_bug(), "{reformatted:#?}");
        assert_eq!(reformatted.statements[2].output_line, 4);
        assert_eq!(reformatted.statements[3].output_line, 6);
        assert_eq!(
            (
                reformatted.counts().same,
                reformatted.counts().changed_judged
            ),
            (4, 1)
        );

        assert!(!unchanged.is_bug());
        assert_eq!(unchanged.counts().changed, 0);
        assert_eq!(unchanged.counts().same, 4);
    }
}

#[test]
fn whole_texts_are_judged_first() {
    let Some(oracle) = oracle() else {
        return;
    };
    let input = "SELECT 1;\nSELECT 2;\n";
    let pair = |output| Pair {
        input,
        output,
        mode: Mode::Sql,
    };
    let reports = oracle.judge_pairs(&[
        pair("SELECT 1;\n\nSELECT 2;\n"),
        pair("SELECT 1;\nSELECT 3;\n"),
        pair("SELECT 1;\nSELECT 2 +;\n"),
        pair("SELECT 1, SELECT 2;\n"),
    ]);
    // The same whole texts make every statement the same.
    assert_eq!(reports[0].whole, Judgement::Same);
    assert_eq!(reports[0].counts().same, 2);
    assert_eq!(reports[0].statements[1].output_line, 3);
    // Differing whole texts are judged statement by statement, to find the
    // statement that differs.
    assert!(matches!(reports[1].whole, Judgement::Differs { .. }));
    assert_eq!(reports[1].statements[0].judgement, Judgement::Same);
    assert!(matches!(
        reports[1].statements[1].judgement,
        Judgement::Differs { .. }
    ));
    // Statements that do not correspond are a bug of their own.
    assert_eq!(reports[2].mismatch, None);
    assert!(reports[2].is_bug());
    assert_eq!(
        reports[3].mismatch,
        Some(Mismatch {
            input: 2,
            output: 1
        })
    );
    assert!(reports[3].is_bug());
    assert_eq!(reports[3].counts().statements, 2);
}
