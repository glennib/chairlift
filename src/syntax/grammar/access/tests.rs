//! Access control statements.

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
    parse(source, Mode::Jinja).statements()[0].outcome
}

#[test]
fn grant_and_revoke() {
    insta::assert_snapshot!(trees(&[
        "GRANT SELECT, INSERT, ALTER UPDATE ON db.t TO a, b WITH GRANT OPTION",
        "grant select(a, b), dictGet on *.* to u with replace option",
        "GRANT ON CLUSTER c ALL ON db.* TO CURRENT_USER, u@'%'",
        "GRANT SELECT ON db.t, INSERT ON db.x TO {{ role }}",
        "GRANT r1, r2 TO u WITH ADMIN OPTION",
        "GRANT CURRENT GRANTS(SELECT ON db.*) TO u",
        "REVOKE GRANT OPTION FOR SELECT ON db.* FROM ALL EXCEPT u",
        "REVOKE ADMIN OPTION FOR r ON CLUSTER c FROM u",
        "GRANT system reload dictionary ON *.* TO u",
    ]));
}

#[test]
fn users_and_roles() {
    insta::assert_snapshot!(trees(&[
        "CREATE USER IF NOT EXISTS u1, u2@'%' ON CLUSTER c IDENTIFIED WITH plaintext_password BY 'secret'",
        "CREATE USER u IDENTIFIED BY 'x' HOST IP '127.0.0.1', LOCAL DEFAULT ROLE ALL EXCEPT r \
         DEFAULT DATABASE db GRANTEES ANY EXCEPT v SETTINGS max_memory_usage = 1 READONLY",
        "CREATE USER u IDENTIFIED WITH ldap SERVER 'srv' VALID UNTIL '2030-01-01' IN local_directory",
        "ALTER USER IF EXISTS u RENAME TO v DROP HOST LOCAL",
        "ALTER USER u DEFAULT ROLE NONE",
        "CREATE ROLE OR REPLACE admin SETTINGS PROFILE 'readonly', max_threads = 1 MIN 1 MAX 8 CONST",
        "ALTER ROLE r ON CLUSTER c RENAME TO s",
        "DROP USER IF EXISTS u, v@'%' ON CLUSTER c",
        "DROP ROLE r FROM local_directory",
        "SET ROLE ALL EXCEPT r",
        "SET ROLE DEFAULT",
        "SET DEFAULT ROLE NONE TO u, CURRENT_USER",
    ]));
}

#[test]
fn quotas_policies_and_profiles() {
    insta::assert_snapshot!(trees(&[
        "CREATE QUOTA q KEYED BY user_name FOR INTERVAL 1 hour MAX queries = 10, errors = 2, \
         FOR RANDOMIZED INTERVAL 2 DAY NO LIMITS TO r",
        "ALTER QUOTA IF EXISTS q FOR INTERVAL 1 MONTH TRACKING ONLY NOT KEYED",
        "CREATE ROW POLICY p ON db.t AS RESTRICTIVE FOR SELECT USING a = 1 AND b TO ALL EXCEPT r",
        "CREATE POLICY p1, p2 ON db.t, db.u USING NONE",
        "DROP ROW POLICY IF EXISTS p ON db.t, q ON db.u ON CLUSTER c",
        "CREATE SETTINGS PROFILE p SETTINGS max_threads = 1 READONLY INHERIT 'default' TO r",
        "ALTER PROFILE p ADD SETTING a = 1",
        "DROP SETTINGS PROFILE IF EXISTS p, 'q'",
        "DROP QUOTA q",
    ]));
}

/// A clause chairlift does not model is kept as written by design, not as
/// a parse failure.
#[test]
fn unmodelled_clauses_are_opaque() {
    for source in [
        "CREATE USER u IDENTIFIED WITH ssh_key BY KEY 'abc' TYPE 'ssh-ed25519'",
        "ALTER USER u ADD PROFILE 'p'",
        "ALTER ROLE r DROP ALL SETTINGS",
    ] {
        assert_eq!(outcome(source), StatementOutcome::Opaque, "{source}");
    }
    for source in [
        "GRANT SELECT ON db.t TO u",
        "CREATE QUOTA q FOR INTERVAL 1 hour MAX queries 10",
        "SET DEFAULT ROLE r TO u",
        "ALTER USER u ADD HOST IP '::1'",
        "ALTER USER u RESET AUTHENTICATION METHODS TO NEW",
    ] {
        assert_eq!(outcome(source), StatementOutcome::Parsed, "{source}");
    }
}

/// Words that are keywords in one place and names in another.
#[test]
fn keyword_slots() {
    for (source, expected) in [
        // `all` is a user in GRANT, everyone in REVOKE.
        ("grant select on t to all", "GRANT SELECT ON t TO all"),
        ("revoke select on t from all", "REVOKE SELECT ON t FROM ALL"),
        (
            "grant select on default.t to default",
            "GRANT SELECT ON default.t TO default",
        ),
        // A privilege of words that are not all keywords keeps its case.
        ("grant dictGet on *.* to u", "GRANT dictGet ON *.* TO u"),
        (
            "grant system reload dictionary on *.* to u",
            "GRANT system reload dictionary ON *.* TO u",
        ),
        (
            "grant alter update on t to u",
            "GRANT ALTER UPDATE ON t TO u",
        ),
        // `currentUser` keeps its case, `current_user` is re-cased.
        (
            "grant select on t to currentUser()",
            "GRANT SELECT ON t TO currentUser()",
        ),
        (
            "grant select on t to current_user",
            "GRANT SELECT ON t TO CURRENT_USER",
        ),
        (
            "create user admin identified with plaintext_password by 'Secret' default database system",
            "CREATE USER admin IDENTIFIED WITH plaintext_password BY 'Secret' DEFAULT DATABASE system",
        ),
        (
            "create quota q keyed by user_name for interval 1 hour max queries = 1",
            "CREATE QUOTA q KEYED BY user_name FOR INTERVAL 1 HOUR MAX queries = 1",
        ),
        (
            "create settings profile p settings max_threads = 1 readonly, profile 'x'",
            "CREATE SETTINGS PROFILE p SETTINGS max_threads = 1 READONLY, PROFILE 'x'",
        ),
        ("set role none", "SET ROLE NONE"),
        // `role` is a setting before `,` or `=`; `true` is no privilege word.
        ("set role, a = 1", "SET role, a = 1"),
        (
            "create user u settings setting = 1, settings b = 2",
            "CREATE USER u SETTINGS setting = 1, SETTINGS b = 2",
        ),
        ("grant alter true on t to u", "GRANT alter true ON t TO u"),
        ("drop role if exists none", "DROP ROLE IF EXISTS none"),
    ] {
        let out = recased(source, Entry::File);
        assert_eq!(out, expected, "{source}");
        verify(source, &out, Mode::Sql, parse(source, Mode::Sql).facts())
            .unwrap_or_else(|e| panic!("{source}: {e}"));
    }
}
