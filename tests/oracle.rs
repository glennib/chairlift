//! The `clickhouse-format` oracle, run for real: batching, the per-text
//! fallback and the classification of failures.
//!
//! `CHAIRLIFT_CLICKHOUSE_FORMAT` names the binary; the default is
//! `clickhouse-format` on `PATH`, which `mise` provides. Without it the tests
//! skip, unless `CHAIRLIFT_REQUIRE_CORPUS` is set.

use chairlift::lexer::Mode;
use chairlift::oracle::Judgement;
use chairlift::oracle::Oracle;
use chairlift::oracle::RejectionClass;
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
