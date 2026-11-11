//! The layout proposal's examples (`docs/spikes/2026-11-10-layout-proposal.md`)
//! through `chairlift::format`: each must come out as the proposal prints it,
//! byte for byte, both from its text flattened (line breaks made spaces,
//! except next to comments and tags and where a blank line is, and keywords
//! in lower case) and from the text itself. `tests/doc_golden.rs` builds the
//! same documents by hand.

use std::collections::HashSet;

use chairlift::Options;
use chairlift::lexer::Mode;
use chairlift::lexer::Token;
use chairlift::lexer::TokenKind;
use chairlift::lexer::tokenize;
use chairlift::syntax::SyntaxKind;

#[path = "support/proposal.rs"]
mod proposal;

/// The expected text with its line breaks made spaces, except where a
/// blank line is and next to comments and block tags, and its upper-case
/// keywords in lower case: the words in keyword slots, which the formatter
/// re-cases, and not a type name such as `UUID` or the words of a statement
/// it keeps as written.
fn flatten(expected: &str, mode: Mode) -> String {
    let tokens = tokenize(expected, mode);
    let slots: HashSet<usize> = chairlift::syntax::parse(expected, mode)
        .syntax()
        .descendants_with_tokens()
        .filter_map(rowan::NodeOrToken::into_token)
        .filter(|t| t.kind() == SyntaxKind::Keyword)
        .map(|t| usize::from(t.text_range().start()))
        .collect();
    let breaks = |t: Option<&Token>| {
        t.is_some_and(|t| t.kind().is_comment() || t.kind() == TokenKind::JinjaBlock)
    };
    let mut out = String::new();
    for (i, token) in tokens.iter().enumerate() {
        let text = token.text(expected);
        match token.kind() {
            TokenKind::Whitespace => {
                let newlines = text.matches('\n').count();
                let near =
                    breaks(i.checked_sub(1).map(|j| &tokens[j])) || breaks(tokens.get(i + 1));
                if newlines >= 2 || (newlines == 1 && near) {
                    out.push_str(&"\n".repeat(newlines));
                } else {
                    out.push(' ');
                }
            }
            TokenKind::BareWord
                if slots.contains(&token.start())
                    && text.bytes().all(|b| !b.is_ascii_lowercase()) =>
            {
                out.push_str(&text.to_ascii_lowercase());
            }
            _ => out.push_str(text),
        }
    }
    out
}

fn format(source: &str, mode: Mode, width: u16) -> String {
    let options = Options::default()
        .with_jinja(mode == Mode::Jinja)
        .with_line_width(width);
    chairlift::format(source, &options).unwrap_or_else(|e| panic!("{e}\n--- input:\n{source}"))
}

/// Checks that `expected` formats to itself at `width`, from its flattened
/// text and from itself.
fn golden_at(expected: &str, mode: Mode, width: u16) {
    let expected = expected.strip_prefix('\n').unwrap_or(expected);
    for source in [flatten(expected, mode), expected.to_owned()] {
        let out = format(&source, mode, width);
        assert_eq!(out, expected, "formatted from:\n{source}");
    }
}

fn golden(expected: &str) {
    golden_at(expected, Mode::Sql, 100);
}

/// Checks that `expected`, whose statements the layout keeps as written,
/// formats to itself.
fn kept(expected: &str) {
    let expected = expected.strip_prefix('\n').unwrap_or(expected);
    assert_eq!(format(expected, Mode::Sql, 100), expected);
}

// Section 2a: line width.

#[test]
fn one_query_at_three_widths() {
    golden_at(proposal::WIDTH_120, Mode::Sql, 120);
    golden_at(proposal::WIDTH_100, Mode::Sql, 100);
    golden_at(proposal::WIDTH_80, Mode::Sql, 80);
}

// Section 2b: lists.

#[test]
fn a_select_list_with_more_than_one_item_always_breaks() {
    golden(proposal::SELECT_LIST_SPLITS);
}

#[test]
fn an_array_literal_goes_one_item_per_line() {
    golden(proposal::ARRAY_LITERAL);
}

// Section 2c: conditions and commas.

#[test]
fn conditions_hang_with_leading_operators() {
    golden(proposal::CONDITIONS_HANG);
}

#[test]
fn commas_trail_and_group_by_fits() {
    golden(proposal::TRAILING_COMMAS);
}

// Section 2e: blank lines.

#[test]
fn a_statement_on_several_lines_gets_a_blank_line_on_each_side() {
    golden(proposal::BLANK_AROUND_MULTILINE);
}

// Section 2f: comments.

#[test]
fn a_line_comment_breaks_the_group_it_is_in() {
    golden(proposal::LINE_COMMENT_BREAKS);
}

#[test]
fn a_comment_pins_a_leading_comma() {
    golden(proposal::PINNED_COMMA);
}

#[test]
fn comments_in_conditions() {
    golden(proposal::CONDITION_COMMENTS);
}

#[test]
fn a_semicolon_after_a_trailing_comment_stands_alone() {
    golden(proposal::SEMICOLON_ALONE);
}

// Section 2g: Jinja.

#[test]
fn block_tags_stay_at_the_indentation_of_the_sql_they_wrap() {
    golden_at(proposal::BLOCK_TAGS, Mode::Jinja, 100);
}

#[test]
fn a_for_loop_stays_at_the_indentation_of_the_list() {
    golden_at(proposal::FOR_LOOP, Mode::Jinja, 100);
}

// Section 2h: nesting.

#[test]
fn a_subquery_hugs_its_brackets() {
    golden(proposal::SUBQUERY_HUGS);
}

#[test]
fn an_in_subquery_hugs_its_brackets() {
    golden(proposal::IN_SUBQUERY_HUGS);
}

#[test]
fn a_single_cte_hugs_with() {
    golden(proposal::SINGLE_CTE_HUGS);
}

#[test]
fn case_expands_with_when_and_else_one_level_in() {
    golden(proposal::CASE_EXPANDS);
}

#[test]
fn a_lambda_stays_whole_when_it_fits() {
    golden(proposal::LAMBDA_WHOLE);
}

#[test]
fn a_lambda_breaks_after_the_arrow() {
    golden_at(proposal::LAMBDA_BREAKS_90, Mode::Sql, 90);
}

#[test]
fn a_join_condition_breaks_before_on_with_leading_and() {
    golden(proposal::JOIN_ON);
}

#[test]
fn insert_select_keeps_its_columns_on_the_insert_line() {
    golden(proposal::INSERT_SELECT);
}

// Section 3: ClickHouse-specific constructs.

#[test]
fn query_clauses_of_clickhouse() {
    golden(proposal::QUERY_CLAUSES);
}

#[test]
fn casts_parameters_and_except_are_atoms() {
    golden(proposal::ATOMS);
}

/// A window specification the parser keeps as written, re-indented as a
/// block so that its second line stays under its first.
#[test]
fn verbatim_text_is_reindented_as_a_block() {
    let source =
        "select\n  a,\n  sum(v) over (partition by k\n               order by ts) as s\nfrom t";
    assert_eq!(
        format(source, Mode::Sql, 100),
        "SELECT\n    a,\n    sum(v) OVER (partition by k\n                 order by ts) AS s\nFROM t\n"
    );
}

// Statements without a layout rule yet are kept as written; written as the
// proposal prints them, they stay so.

#[test]
fn statements_kept_as_written_stay_as_the_proposal_prints_them() {
    for expected in [
        proposal::IN_LIST,
        proposal::BLANK_LINES_KEPT,
        proposal::ALTER_TABLE,
        proposal::GRANT,
    ] {
        kept(expected);
    }
}

// The `CREATE` examples of sections 2 and 3, and the DESIGN.md example of
// section 4.

#[test]
fn create_table_examples() {
    for expected in [
        proposal::SETTINGS_LIST,
        proposal::ORDER_BY_FITS,
        proposal::ORDER_BY_BREAKS,
        proposal::COLUMNS_NOT_ALIGNED,
        proposal::COLUMN_COMMENTS,
        proposal::PROJECTION_AND_INDEX,
        proposal::STORAGE_CLAUSES,
    ] {
        golden(expected);
    }
}

#[test]
fn a_materialized_view_ends_its_header_with_as() {
    golden(proposal::MATERIALIZED_VIEW);
}

#[test]
fn the_design_example_prints_as_the_proposal_says() {
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/design/");
    let input = std::fs::read_to_string(format!("{fixtures}page_views_1h.sql.j2")).expect("input");
    let expected = std::fs::read_to_string(format!("{fixtures}page_views_1h.expected.sql.j2"))
        .expect("expected output");
    assert_eq!(format(&input, Mode::Jinja, 100), expected);
    golden_at(&expected, Mode::Jinja, 100);
}

// Statements the proposal has no example of, laid out by its rules: the
// header on one line, lists that fit or go one item per line, a query at
// column 0, and blank lines around a statement on several lines.

const INSERTS: &str = "
INSERT INTO analytics.sites (site_id, name, timezone) VALUES (1, 'Nordlys', 'Europe/Oslo');
INSERT INTO analytics.sites SETTINGS async_insert = 1 VALUES (2, 'Fjordposten', 'Europe/Oslo');

INSERT INTO TABLE FUNCTION file('sites.csv', 'CSVWithNames')
SELECT *
FROM analytics.sites;

WITH recent AS (
    SELECT *
    FROM raw.events
    WHERE event_time > now() - INTERVAL 1 DAY
)
INSERT INTO analytics.recent_sites (site, views)
SELECT
    site,
    count()
FROM recent
GROUP BY site;

INSERT INTO analytics.page_views_by_site_and_hour (
    site,
    hour,
    page_views,
    unique_visitors,
    sessions,
    bounce_rate
)
SELECT *
FROM staging.page_views_by_site_and_hour;
";

#[test]
fn inserts_in_the_proposal_s_style() {
    golden(INSERTS);
}
