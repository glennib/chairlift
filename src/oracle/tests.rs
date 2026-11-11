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

#[test]
fn long_canonical_forms_are_cut_around_the_difference() {
    let a = format!("SELECT {}1", "x, ".repeat(100));
    let b = format!("SELECT {}2", "x, ".repeat(100));
    let message = difference(&a, &b);
    assert!(message.contains("\"… x, x, "), "{message}");
    assert!(message.contains("x, 1\" for the input"), "{message}");
    assert!(message.contains("x, 2\" for the output"), "{message}");
    let long = format!("{a}{}", " AND y".repeat(100));
    assert!(difference(&long, &b).contains("…\" for the input"));
}

#[test]
fn a_file_is_its_own_variant() {
    let found = variants("select 1", "SELECT 1\n", Mode::Jinja);
    // With the `template` feature, followed by its one render.
    assert_eq!(found.len(), if cfg!(feature = "template") { 2 } else { 1 });
    assert_eq!(found[0].rendering, None);
    assert_eq!(found[0].input, "select 1");
    assert_eq!(found[0].output, "SELECT 1\n");
    assert_eq!(found[0].mode, Mode::Jinja);
}

/// With the `template` feature, a template is also judged once per distinct
/// render: here one per branch.
#[cfg(feature = "template")]
#[test]
fn a_template_is_judged_per_render() {
    let input = "select 1{% if x %}, 2{% endif %}";
    let output = "SELECT\n    1{% if x %}, 2{% endif %}\n";
    let found = variants(input, output, Mode::Jinja);
    let renders: Vec<(&str, &str, Mode)> = found[1..]
        .iter()
        .map(|v| (v.input.as_ref(), v.output.as_ref(), v.mode))
        .collect();
    assert_eq!(
        renders,
        [
            ("select 1, 2", "SELECT\n    1, 2\n", Mode::Sql),
            ("select 1", "SELECT\n    1\n", Mode::Sql),
        ]
    );
    assert_eq!(found[1].rendering.as_deref(), Some("render branches 0"));
    // SQL is not rendered.
    assert_eq!(variants(input, output, Mode::Sql).len(), 1);
}

#[test]
fn output_is_read_up_to_a_limit() {
    for drain in [false, true] {
        let mut reader = &b"abcdef"[..];
        let (kept, more) = read_limited(&mut reader, 4, drain);
        assert_eq!((kept.as_slice(), more), (&b"abcd"[..], true));
        assert_eq!(reader.len(), usize::from(!drain));
        let (kept, more) = read_limited(&mut &b"abcd"[..], 4, drain);
        assert_eq!((kept.as_slice(), more), (&b"abcd"[..], false));
    }
}

#[test]
fn the_single_binary_formats_as_a_subcommand() {
    assert!(is_multi_call(Path::new("/opt/ch/bin/clickhouse")));
    assert!(is_multi_call(Path::new("clickhouse.exe")));
    assert!(!is_multi_call(Path::new("clickhouse-format")));
    assert!(!is_multi_call(Path::new("/usr/bin/clickhouse-local")));
}

#[test]
fn a_missing_program_is_reported() {
    let oracle = Oracle::new("/nonexistent/clickhouse-format");
    assert_eq!(
        oracle.probe(),
        Err(ProbeError::NotFound {
            program: "/nonexistent/clickhouse-format".to_owned()
        })
    );
    assert!(matches!(
        oracle.canonical(&["select 1"])[0],
        Verdict::Rejected(Rejection {
            class: RejectionClass::Failed,
            ..
        })
    ));
}

/// Programs that misbehave: the wrong program, one that hangs, and ones that
/// print without end or too much. None may hang the oracle or fill memory.
#[cfg(unix)]
mod misbehaving {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::PathBuf;

    use super::*;

    /// A shell script running `body`, in a fresh directory.
    fn script(name: &str, body: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("chairlift-oracle-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clickhouse-format");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn failure(oracle: &Oracle) -> String {
        match &oracle.canonical(&["select 1"])[0] {
            Verdict::Rejected(Rejection {
                class: RejectionClass::Failed,
                message,
            }) => message.clone(),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn the_wrong_program() {
        let oracle = Oracle::new(script("echo", "echo hello"));
        assert!(matches!(
            oracle.probe(),
            Err(ProbeError::Unexpected { what, .. }) if what == "prints \"hello\""
        ));
        let oracle = Oracle::new(script("fails", "echo 'no such option' >&2; exit 3"));
        assert!(matches!(
            oracle.probe(),
            Err(ProbeError::Unexpected { what, .. }) if what == "fails with \"no such option\""
        ));
    }

    #[test]
    fn a_program_that_cannot_be_run() {
        let path = script("not-executable", "exit 0");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            Oracle::new(path).probe(),
            Err(ProbeError::CannotRun { .. })
        ));
    }

    #[test]
    fn a_hanging_program_is_killed() {
        let mut oracle = Oracle::new(script("hangs", "exec sleep 30"));
        oracle.timeout = Duration::from_millis(300);
        let started = Instant::now();
        assert!(failure(&oracle).starts_with("timed out"));
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(matches!(oracle.probe(), Err(ProbeError::CannotRun { .. })));
    }

    /// A wrapper script whose child hangs: killing the script leaves the
    /// child holding the pipes, which must not hold up the oracle.
    #[test]
    fn a_hanging_child_of_the_program_is_not_waited_for() {
        let mut oracle = Oracle::new(script("hanging-child", "sleep 10"));
        oracle.timeout = Duration::from_millis(300);
        let started = Instant::now();
        assert!(failure(&oracle).starts_with("timed out"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// A program that exits and leaves a process behind that holds its
    /// output open gives no verdict.
    #[test]
    fn output_held_open_after_exit_is_a_failure() {
        let oracle = Oracle::new(script("background", "sleep 10 &\necho 'SELECT 1'"));
        let started = Instant::now();
        assert!(failure(&oracle).contains("holds its output open"));
        assert!(started.elapsed() < Duration::from_secs(9));
    }

    /// A program that prints without end is stopped as soon as it passes the
    /// limit, not at the timeout.
    #[test]
    fn endless_output_fails_at_once() {
        let mut oracle = Oracle::new(script("endless-fast", "exec yes"));
        oracle.timeout = Duration::from_secs(30);
        let started = Instant::now();
        assert!(failure(&oracle).starts_with("printed more than"));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    /// Whether the process `pid` has gone, or is a zombie waiting to be
    /// reaped; waits a little for it.
    fn is_gone(pid: &str) -> bool {
        let stat = format!("/proc/{pid}/stat");
        for _ in 0..100 {
            match fs::read_to_string(&stat) {
                Err(_) => return true,
                Ok(s)
                    if s.rsplit(')')
                        .next()
                        .is_some_and(|r| r.trim_start().starts_with('Z')) =>
                {
                    return true;
                }
                Ok(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
        false
    }

    /// A process the program started is killed with it on a timeout, also
    /// one that left the program's process.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_timeout_kills_what_the_program_started() {
        let pids =
            std::env::temp_dir().join(format!("chairlift-oracle-{}-pids", std::process::id()));
        let _ = fs::remove_file(&pids);
        let mut oracle = Oracle::new(script(
            "grandchild",
            &format!("(sleep 1000 & echo $! > '{}'); sleep 1000", pids.display()),
        ));
        oracle.timeout = Duration::from_millis(300);
        assert!(failure(&oracle).starts_with("timed out"));
        let pid = fs::read_to_string(&pids).unwrap();
        let _ = fs::remove_file(&pids);
        assert!(is_gone(pid.trim()), "process {pid} is still running");
    }

    /// After a few timeouts in a row, the program is not run any more: a
    /// program that hangs on every input costs a few timeouts, not one per
    /// text.
    #[test]
    fn a_program_that_keeps_timing_out_is_given_up() {
        let mut oracle = Oracle::new(script("hangs-always", "exec sleep 30"));
        oracle.timeout = Duration::from_millis(200);
        oracle.jobs = 1;
        let started = Instant::now();
        let verdicts = oracle.canonical(&["select 1"; 20]);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(oracle.gave_up());
        let messages: Vec<&str> = verdicts
            .iter()
            .map(|v| match v {
                Verdict::Rejected(Rejection {
                    class: RejectionClass::Failed,
                    message,
                }) => message.as_str(),
                other => panic!("expected a failure, got {other:?}"),
            })
            .collect();
        assert!(
            messages.iter().any(|m| m.starts_with("not run")),
            "{messages:?}"
        );
        // A program that times out only now and then is not given up.
        let mut oracle = Oracle::new(script(
            "hangs-sometimes",
            r#"input=$(cat); case "$input" in *hang*) exec sleep 30 ;; esac; echo 'SELECT 1;'"#,
        ));
        oracle.timeout = Duration::from_millis(200);
        oracle.jobs = 1;
        oracle.batch_files = 1;
        let verdicts = oracle.canonical(&["hang", "select 1", "hang", "select 1", "hang"]);
        assert!(!oracle.gave_up());
        assert_eq!(verdicts[3], Verdict::Accepted("SELECT 1;\n".to_owned()));
    }

    #[test]
    fn too_much_output_fails() {
        let oracle = Oracle::new(script("too-much", "head -c 3000000 /dev/zero"));
        assert!(failure(&oracle).starts_with("printed more than"));
    }
}
