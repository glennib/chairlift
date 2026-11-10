//! `chairlift::format` on the vendored fixtures.

use std::fs;
use std::path::Path;

use chairlift::Options;
use chairlift::lexer::Mode;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn options_for(name: &str) -> Options {
    Options::default().with_jinja(Mode::for_path(Path::new(name)) == Mode::Jinja)
}

/// The DESIGN.md example has no trailing whitespace or runs of blank lines,
/// so the cleanup leaves it as it is; both it and its expected layout are
/// formatted already as far as the cleanup goes.
#[test]
fn design_example_is_unchanged() {
    for name in [
        "design/page_views_1h.sql.j2",
        "design/page_views_1h.expected.sql.j2",
    ] {
        let source = fixture(name);
        assert_eq!(
            chairlift::format(&source, &options_for(name)).unwrap(),
            source,
            "{name}"
        );
        assert!(chairlift::check(&source, &options_for(name)).unwrap());
    }
}

#[test]
fn check_reports_whether_a_text_is_formatted() {
    let options = Options::default();
    assert!(chairlift::check("SELECT 1;\n", &options).unwrap());
    assert!(!chairlift::check("SELECT 1; \n", &options).unwrap());
    assert!(!chairlift::check("SELECT 1;", &options).unwrap());
    assert!(chairlift::check("SELECT 'x", &options).is_err());
}

#[test]
fn line_width_is_accepted_and_does_nothing_yet() {
    let source = "SELECT a, b, c FROM t\n";
    let narrow = Options::default().with_line_width(10);
    assert_eq!(narrow.line_width, 10);
    assert_eq!(chairlift::format(source, &narrow).unwrap(), source);
}
