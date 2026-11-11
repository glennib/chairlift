use super::*;

#[test]
fn multi_line_values_are_joined() {
    assert_eq!(
        prepare(
            "INSERT INTO t VALUES (1, 'a'),\n  -- two\n  (2, 'b\nc') /* x\n */;\nSELECT\n1;",
            Mode::Sql
        ),
        "INSERT INTO t VALUES (1, 'a'),   (2, 'b\nc')  ;\nSELECT\n1;"
    );
    // One-line rows, other data and other statements stay as they are.
    for text in [
        "INSERT INTO t VALUES (1), (2);\n",
        "INSERT INTO t FORMAT CSV\n1\n2\n\n",
        "SELECT\n1;",
    ] {
        assert_eq!(prepare(text, Mode::Sql), text);
    }
}

#[test]
fn texts_in_a_batch_end_their_last_statement() {
    assert_eq!(terminator("SELECT 1;"), "\n");
    assert_eq!(terminator("SELECT 1"), ";\n");
    assert_eq!(terminator("INSERT INTO t VALUES (1)"), ";\n");
    assert_eq!(terminator("SELECT 1 -- c"), "\n;\n");
    assert_eq!(terminator("-- only a comment"), "\n");
    assert_eq!(terminator(""), "\n");
}

#[test]
fn batch_output_is_split_at_boundaries() {
    let stdout = format!(
        "SELECT 1;\n\nSELECT 2;\n\n{b0}\n\n{b1}\n\nCREATE TABLE t;\n\n",
        b0 = boundary(0).trim_end(),
        b1 = boundary(1).trim_end()
    );
    assert_eq!(split_batch_output(&stdout), ["SELECT 1;\nSELECT 2;\n", ""]);
}

#[test]
fn errors_are_classified() {
    let case = |stderr: &str| classify(stderr).class;
    assert_eq!(
        case(
            "Code: 62. DB::Exception: Syntax error (query): failed at position 9 ({): { x }};. \
             Expected substitution name (identifier). (SYNTAX_ERROR), Stack trace (when copying \
             this message, always include the lines below):\n\n0. DB::Exception…"
        ),
        RejectionClass::UnrenderedJinja
    );
    assert_eq!(
        case("Code: 62. DB::Exception: Syntax error (query): failed at position 12 (%): % if"),
        RejectionClass::UnrenderedJinja
    );
    assert_eq!(
        case(
            "Code: 48. DB::Exception: Can't format INSERT query with data format 'CSV'. (NOT_IMPLEMENTED)"
        ),
        RejectionClass::InsertData
    );
    assert_eq!(
        case("Code: 62. DB::Exception: Empty query (query). (SYNTAX_ERROR)"),
        RejectionClass::EmptyQuery
    );
    assert_eq!(
        case("Code: 62. DB::Exception: Syntax error (query): failed at position 9 (end of query)"),
        RejectionClass::Syntax
    );
    assert_eq!(
        case("Code: 36. DB::Exception: Multiple primary keys are not allowed. (BAD_ARGUMENTS)"),
        RejectionClass::Other
    );
    assert_eq!(case("Segmentation fault"), RejectionClass::Failed);
    assert_eq!(case(""), RejectionClass::Failed);
    let rejection = classify(
        "Code: 62. DB::Exception: Empty query (query). (SYNTAX_ERROR), Stack trace (when copying \
         this message, always include the lines below):\n\n0. x",
    );
    assert_eq!(
        rejection.message,
        "Code: 62. DB::Exception: Empty query (query). (SYNTAX_ERROR)"
    );
}

#[test]
fn classes_index_their_counts() {
    // `Counts` indexes its rejections by discriminant.
    for (i, class) in RejectionClass::ALL.into_iter().enumerate() {
        assert_eq!(class as usize, i);
    }
    let rejected = |class: RejectionClass| StatementReport {
        line: 1,
        output_line: 1,
        changed: true,
        judgement: Judgement::Unjudgeable(Rejection {
            class,
            message: String::new(),
        }),
    };
    let report = Report {
        whole: Judgement::Same,
        statements: vec![
            rejected(RejectionClass::Syntax),
            rejected(RejectionClass::Failed),
            rejected(RejectionClass::Syntax),
        ],
        mismatch: None,
    };
    let mut counts = report.counts();
    counts.add(&report.counts());
    assert_eq!(counts.statements, 6);
    assert_eq!(counts.changed, 6);
    assert_eq!(counts.changed_judged, 0);
    assert_eq!(counts.judged(), 0);
    assert_eq!(counts.rejected(RejectionClass::Syntax), 4);
    assert_eq!(counts.rejected(RejectionClass::Failed), 2);
    assert_eq!(counts.rejected_total(), 6);
}

#[test]
fn judgements() {
    let yes = |s: &str| Verdict::Accepted(s.to_owned());
    let no = Verdict::Rejected(Rejection {
        class: RejectionClass::Syntax,
        message: "x".to_owned(),
    });
    assert_eq!(judge(&yes("a"), &yes("a")), Judgement::Same);
    assert!(matches!(
        judge(&yes("a"), &yes("b")),
        Judgement::Differs { .. }
    ));
    assert!(matches!(
        judge(&yes("a"), &no),
        Judgement::OutputRejected(_)
    ));
    assert!(matches!(judge(&no, &yes("a")), Judgement::Unjudgeable(_)));
    assert!(matches!(judge(&no, &no), Judgement::Unjudgeable(_)));
    // A crash or a timeout on the output is no evidence against it.
    let failed = Verdict::Rejected(Rejection {
        class: RejectionClass::Failed,
        message: "timed out after 60s".to_owned(),
    });
    assert!(matches!(
        judge(&yes("a"), &failed),
        Judgement::Unjudgeable(Rejection {
            class: RejectionClass::Failed,
            ..
        })
    ));
}
