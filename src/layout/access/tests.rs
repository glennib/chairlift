use crate::Options;

/// Formats `source` at `width`, checking that formatting again changes
/// nothing.
fn fmt_in(source: &str, width: u16) -> String {
    let options = Options::default().with_line_width(width).with_jinja(true);
    let once = crate::format(source, &options).unwrap_or_else(|e| panic!("{source:?}: {e}"));
    let twice = crate::format(&once, &options).unwrap_or_else(|e| panic!("{once:?}: {e}"));
    assert_eq!(twice, once, "not idempotent on {source:?}");
    once
}

/// Checks that `source` formats to `expected` at `width`, and `expected`
/// to itself.
fn check_at(source: &str, expected: &str, width: u16) {
    let expected = expected.strip_prefix('\n').unwrap_or(expected);
    assert_eq!(fmt_in(source, width), expected, "from {source:?}");
    assert_eq!(fmt_in(expected, width), expected);
}

fn check(source: &str, expected: &str) {
    check_at(source, expected, 100);
}

#[test]
fn statements_that_fit_stay_on_one_line() {
    check(
        "grant select(a,b), dictGet on *.* to u@'%', currentUser() with replace option;\n\
         grant  select on db.t, insert on db.x to {{ role }};\n\
         revoke grant option for select on db.* from all except u;\n\
         grant r1, r2 to u with admin option;\n\
         create row policy p on db.t as restrictive for select using a = 1 to all except r;\n\
         set role all except r;\nset default role none to u;",
        "
GRANT SELECT(a, b), dictGet ON *.* TO u@'%', currentUser() WITH REPLACE OPTION;
GRANT SELECT ON db.t, INSERT ON db.x TO {{ role }};
REVOKE GRANT OPTION FOR SELECT ON db.* FROM ALL EXCEPT u;
GRANT r1, r2 TO u WITH ADMIN OPTION;
CREATE ROW POLICY p ON db.t AS RESTRICTIVE FOR SELECT USING a = 1 TO ALL EXCEPT r;
SET ROLE ALL EXCEPT r;
SET DEFAULT ROLE NONE TO u;
",
    );
}

/// A `GRANT` that does not fit breaks before `ON`, `TO` and `WITH GRANT
/// OPTION`; its lists go one item per line.
#[test]
fn a_grant_that_does_not_fit() {
    check_at(
        "grant select, insert on analytics.events to writer, backfill with grant option",
        "
GRANT SELECT, INSERT
    ON analytics.events
    TO writer, backfill
    WITH GRANT OPTION
",
        50,
    );
    check_at(
        "grant select, insert, alter update on analytics.events to writer",
        "
GRANT
    SELECT,
    INSERT,
    ALTER UPDATE
    ON analytics.events
    TO writer
",
        30,
    );
    check_at(
        "grant select on db.a, insert on db.b to writer",
        "
GRANT
    SELECT ON db.a,
    INSERT ON db.b
    TO writer
",
        30,
    );
}

/// A `CREATE` or `ALTER` that does not fit puts each clause on a line.
#[test]
fn an_entity_that_does_not_fit() {
    check(
        "create user if not exists u1 identified with plaintext_password by 'Secret!' \
         host ip '127.0.0.1' default role all settings max_memory_usage = 10000000 readonly \
         default database db;\n\
         create settings profile p settings max_threads = 1 readonly, max_memory_usage = 100 \
         min 1 max 1000 inherit 'default' to r;",
        "
CREATE USER IF NOT EXISTS u1
    IDENTIFIED WITH plaintext_password BY 'Secret!'
    HOST IP '127.0.0.1'
    DEFAULT ROLE ALL
    SETTINGS max_memory_usage = 10000000 READONLY
    DEFAULT DATABASE db;

CREATE SETTINGS PROFILE p
    SETTINGS max_threads = 1 READONLY, max_memory_usage = 100 MIN 1 MAX 1000
    INHERIT 'default'
    TO r;
",
    );
}

/// Passwords, hashes and salts are string literals and come out byte for
/// byte, also where a clause is kept as written.
#[test]
fn secrets_are_kept_byte_for_byte() {
    for secret in [
        "'Pa ss;  -- word'",
        r"'\'quoted\' \\ x'",
        "$$heredoc Secret$$",
    ] {
        for source in [
            format!("create user u identified with plaintext_password by {secret}"),
            format!("create user u identified with sha256_hash by {secret} salt {secret}"),
            format!("alter user u add identified with sha256_password by {secret}"),
        ] {
            let out = fmt_in(&source, 100);
            assert!(out.contains(secret), "{source:?} became {out:?}");
        }
    }
}

/// A comment between clauses stays with the clause after it.
#[test]
fn comments_between_clauses() {
    check(
        "create user u -- who\n identified by 'x' default role r",
        "
CREATE USER u  -- who
    IDENTIFIED BY 'x'
    DEFAULT ROLE r
",
    );
}
