//! The shared clause parts.

use crate::syntax::grammar::common;
use crate::syntax::grammar::fallback;
use crate::syntax::parser::Parser;
use crate::syntax::test_support::grammar;

fn order_by(p: &mut Parser<'_>) -> Option<()> {
    common::order_by_list(p, &fallback::never).map(drop)
}

fn settings(p: &mut Parser<'_>) -> Option<()> {
    if !common::at_settings(p) {
        return None;
    }
    common::settings_clause(p, &fallback::never).map(drop)
}

#[expect(clippy::unnecessary_wraps, reason = "the grammar functions' signature")]
fn head(p: &mut Parser<'_>) -> Option<()> {
    common::if_not_exists(p);
    common::if_exists(p);
    common::on_cluster(p);
    Some(())
}

fn trees(inputs: &[&str], parse: fn(&mut Parser<'_>) -> Option<()>) -> String {
    let mut out = String::new();
    for input in inputs {
        out.push_str("> ");
        out.push_str(input);
        out.push('\n');
        out.push_str(&grammar(input, parse));
        out.push('\n');
    }
    out
}

#[test]
fn order_by_items() {
    insta::assert_snapshot!(trees(
        &[
            "a, b DESC",
            "x ASC NULLS FIRST, y COLLATE 'en'",
            "d WITH FILL FROM 1 TO 10 STEP 1",
            "t WITH FILL STEP INTERVAL 1 DAY STALENESS 2",
            "a AS b DESC",
            "f(x) desc, g(y y), z",
        ],
        order_by
    ));
}

#[test]
fn settings_lists() {
    insta::assert_snapshot!(trees(
        &[
            "SETTINGS max_threads = 8, join_algorithm = 'hash'",
            "SETTINGS a.b = 1, c = DEFAULT, d = true, e = -1",
            "SETTINGS x = {'k': 1}, disk = disk(type = s3)",
        ],
        settings
    ));
    // `settings` names a column unless a setting and `=` follow.
    assert_eq!(
        grammar("SETTINGS[1]", settings),
        "VerbatimStatement\n  BareWord \"SETTINGS\"\n  OpeningSquareBracket \"[\"\n  Number \"1\"\n  ClosingSquareBracket \"]\"\n"
    );
}

#[test]
fn statement_heads() {
    insta::assert_snapshot!(trees(
        &[
            "IF NOT EXISTS",
            "IF EXISTS ON CLUSTER '{cluster}'",
            "ON CLUSTER main",
            "ON CLUSTER {{ cluster }}",
        ],
        head
    ));
}
