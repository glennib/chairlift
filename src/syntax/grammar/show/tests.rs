//! `SHOW` and `DESCRIBE`.

use std::fmt::Write as _;

use crate::lexer::Mode;
use crate::syntax::Entry;
use crate::syntax::StatementOutcome;
use crate::syntax::parse;
use crate::syntax::test_support::recased;
use crate::syntax::test_support::tree;
use crate::verify::verify;

/// Each input and its tree.
fn trees(inputs: &[&str]) -> String {
    let mut out = String::new();
    for input in inputs {
        let _ = writeln!(out, "> {input}");
        out.push_str(&tree(input, Entry::File));
        out.push('\n');
    }
    out
}

/// The outcome of the first statement of `source`.
fn outcome(source: &str) -> StatementOutcome {
    parse(source, Mode::Sql).statements()[0].outcome
}

#[test]
fn show() {
    insta::assert_snapshot!(trees(&[
        "SHOW CREATE TABLE db.t FORMAT TSVRaw",
        "show create temporary table t",
        "SHOW CREATE t",
        "SHOW CREATE USER u",
        "SHOW TABLES FROM system LIKE '%a%' LIMIT 2",
        "SHOW FULL TEMPORARY TABLES IN db NOT ILIKE 'x'",
        "SHOW DATABASES",
        "SHOW EXTENDED COLUMNS FROM t FROM db",
        "SHOW INDEX FROM t WHERE key_name = 'x'",
        "SHOW SETTING max_threads",
        "SHOW CHANGED SETTINGS LIKE 'max%'",
        "SHOW GRANTS FOR u, CURRENT_USER WITH IMPLICIT FINAL",
        "SHOW PROCESSLIST INTO OUTFILE 'x'",
    ]));
}

#[test]
fn describe() {
    insta::assert_snapshot!(trees(&[
        "DESCRIBE TABLE db.t",
        "desc t format TSV",
        "DESC TEMPORARY TABLE t",
        "DESCRIBE (SELECT 1 AS x) SETTINGS describe_compact_output = 1",
        "DESCRIBE TABLE file('a.csv', 'CSV')",
        "DESC {{ ref('t') }}",
    ]));
}

#[test]
fn opaque_and_verbatim() {
    assert_eq!(outcome("SHOW CREATE QUOTA q"), StatementOutcome::Opaque);
    assert_eq!(
        outcome("SHOW CREATE ROW POLICY p ON db.t"),
        StatementOutcome::Parsed
    );
    assert_eq!(
        outcome("SHOW TABLES OF something"),
        StatementOutcome::Opaque
    );
    assert_eq!(outcome("SHOW ACCESS"), StatementOutcome::Verbatim);
    assert_eq!(
        outcome("DESC FILESYSTEM CACHE 'c'"),
        StatementOutcome::Verbatim
    );
}

#[test]
fn keyword_slots() {
    for (source, expected) in [
        ("show tables from system", "SHOW TABLES FROM system"),
        ("show create table format", "SHOW CREATE TABLE format"),
        ("show create view", "SHOW CREATE view"),
        ("show create user.t", "SHOW CREATE user.t"),
        ("describe table", "DESCRIBE table"),
        ("desc table t", "DESC TABLE t"),
    ] {
        let out = recased(source, Entry::File);
        assert_eq!(out, expected, "{source}");
        verify(source, &out, Mode::Sql, parse(source, Mode::Sql).facts())
            .unwrap_or_else(|e| panic!("{source}: {e}"));
    }
}
