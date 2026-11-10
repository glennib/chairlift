//! Property tests of the lexer's guarantees, over random text and over text
//! assembled from the fragments where ClickHouse's and Jinja's rules interact.

use chairlift::lexer::Mode;
use chairlift::lexer::Token;
use chairlift::lexer::TokenKind;
use chairlift::lexer::can_touch;
use chairlift::lexer::jinja;
use chairlift::lexer::tokenize;
use chairlift::split::split;
use chairlift::verify::Facts;
use chairlift::verify::verify;
use proptest::prelude::*;

/// Pieces of SQL and Jinja syntax whose rules depend on what is next to them.
#[rustfmt::skip]
const FRAGMENTS: &[&str] = &[
    " ", "\n", "\t", "\r\n", "\u{a0}", "\u{feff}", "\u{2003}", "a", "x", "b", "e", "E", "p",
    "SELECT", "from", "_", "$", "$$", "$a$", "0", "1", "9", "0x", "0b", "1e", "1e+", ".", ".5",
    "5", "-", "--", "+", "*", "/", "/*", "*/", "#", "# ", "#!", "//", "'", "''", "\\", "\"", "`",
    "‘", "’", "“", "”", "\u{2212}", "x'", "b'", "(", ")", "[", "]", "{", "}", ",", ";", ":",
    "::", "=", "!", "!=", "<", ">", "<=>", "|", "||", "|>", "~", "~*", "@", "@@", "?", "^", "%",
    "\\G", "{{", "}}", "{%", "%}", "{#", "#}", "-}}", "{{-", "{%-", "-%}", "{%+", "+%}", "raw",
    "endraw", "{% raw %}", "{% endraw %}", "{{ x }}", "{% if a %}", "{% endif %}", "{# c #}",
    "é", "🙂",
];

fn fragments() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(FRAGMENTS), 0..48).prop_map(|v| v.concat())
}

fn inputs() -> impl Strategy<Value = String> {
    prop_oneof![fragments(), "[ -~\n\t]{0,64}", any::<String>(),]
}

/// Pieces of statements, for the splitter and the formatter: `INSERT` heads
/// and their data, among the lexer's fragments.
#[rustfmt::skip]
const STATEMENT_FRAGMENTS: &[&str] = &[
    "INSERT INTO t ", "insert into ", "VALUES ", "FORMAT ", "CSV", "Values", " (a, b) ", "SELECT ",
    "input('a Int8')", "FROM ", "INFILE 'f' ", "WITH ", "EXPLAIN ", "FUNCTION f(1) ", "(1)", "(2, 'x')",
    ", ", ";", "; ", "\n", "\n\n", "\n\n\n", "  \n", "\t\n", "\r\n", "\r\n\r\n", " ", "\\N", "-- c",
    "-- c  ", "# ", "/* c\n */", "'a\nb'", "SETTINGS x = 1 ", "=", "\u{feff}", "\u{a0}",
];

fn statement_inputs() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        3 => prop::sample::select(STATEMENT_FRAGMENTS),
        1 => prop::sample::select(FRAGMENTS),
    ];
    prop::collection::vec(piece, 0..40).prop_map(|v| v.concat())
}

fn modes() -> impl Strategy<Value = Mode> {
    prop_oneof![Just(Mode::Sql), Just(Mode::Jinja)]
}

fn check_lossless(source: &str, tokens: &[Token]) -> Result<(), TestCaseError> {
    let mut pos = 0;
    for token in tokens {
        prop_assert_eq!(token.start(), pos, "{:?} starts after a gap", token);
        prop_assert!(token.end() > token.start(), "{:?} is empty", token);
        prop_assert!(source.is_char_boundary(token.end()));
        prop_assert!(token.keyword().is_none() || token.kind() == TokenKind::BareWord);
        pos = token.end();
    }
    prop_assert_eq!(pos, source.len());
    Ok(())
}

/// Removes every whitespace token and joins the rest back together the way a
/// printer would: tokens touch where [`can_touch`] allows it, and are
/// separated by a space otherwise, or a newline after a line comment.
///
/// Leading and trailing whitespace each stay as one space: ClickHouse lexes
/// the first and last bytes of the input specially (`.` at the very start is
/// a number, `1e` at the very end is a bare word), so removing what surrounds
/// them can change them.
fn squash(source: &str, tokens: &[Token], mode: Mode) -> String {
    let mut out = String::new();
    if tokens
        .first()
        .is_some_and(|t| t.kind() == TokenKind::Whitespace)
    {
        out.push(' ');
    }
    let trailing = tokens.len() > 1 && tokens.last().unwrap().kind() == TokenKind::Whitespace;
    let mut prev_significant = None;
    let mut last: Option<Token> = None;
    for &token in tokens.iter().filter(|t| t.kind() != TokenKind::Whitespace) {
        if let Some(left) = last {
            if left.kind() == TokenKind::LineComment {
                out.push('\n');
            } else if !can_touch(
                mode,
                prev_significant,
                (left.kind(), left.text(source)),
                (token.kind(), token.text(source)),
            ) {
                out.push(' ');
            }
            if left.kind().is_significant() {
                prev_significant = Some(left.kind());
            }
        }
        out.push_str(token.text(source));
        last = Some(token);
    }
    if trailing {
        out.push(
            if last.is_some_and(|t| t.kind() == TokenKind::LineComment) {
                '\n'
            } else {
                ' '
            },
        );
    }
    out
}

fn without_whitespace<'s>(source: &'s str, tokens: &[Token]) -> Vec<(TokenKind, &'s str)> {
    tokens
        .iter()
        .filter(|t| t.kind() != TokenKind::Whitespace)
        .map(|t| (t.kind(), t.text(source)))
        .collect()
}

proptest! {
    // Failures are not persisted: a case worth keeping goes into the
    // edge-case table in tests/lexer.rs.
    #![proptest_config(ProptestConfig {
        cases: 2048,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Tokens are non-empty, contiguous, on `char` boundaries, and cover the
    /// input; lexing never panics.
    #[test]
    fn tokens_cover_the_input(source in inputs(), mode in modes()) {
        let tokens = tokenize(&source, mode);
        check_lossless(&source, &tokens)?;
        let joined: String = tokens.iter().map(|t| t.text(&source)).collect();
        prop_assert_eq!(joined, source);
    }

    /// SQL mode never produces a Jinja kind.
    #[test]
    fn sql_mode_has_no_jinja(source in inputs()) {
        for token in tokenize(&source, Mode::Sql) {
            prop_assert!(!token.kind().is_jinja());
            prop_assert!(!matches!(
                token.kind(),
                TokenKind::ErrorJinjaTagIsNotClosed | TokenKind::ErrorJinjaRawIsNotClosed
            ));
        }
    }

    /// A printer that drops all whitespace and spaces tokens only where
    /// [`can_touch`] forbids them to touch produces the same tokens.
    #[test]
    fn can_touch_keeps_tokens_apart(source in inputs(), mode in modes()) {
        // A formatter leaves input with error tokens alone, so only the text
        // before the first error is of interest.
        let error = tokenize(&source, mode).into_iter().find(|t| t.kind().is_error());
        let source = &source[..error.map_or(source.len(), Token::start)];
        let tokens = tokenize(source, mode);
        prop_assume!(!tokens.iter().any(|t| t.kind().is_error()));
        let squashed = squash(source, &tokens, mode);
        let again = tokenize(&squashed, mode);
        prop_assert_eq!(
            without_whitespace(&squashed, &again),
            without_whitespace(source, &tokens),
            "squashed to {:?}", squashed
        );
    }

    /// The tags found inside the tokens are the tags found by scanning the
    /// whole text: no token swallows part of a tag or hides one.
    #[test]
    fn tokens_hold_whole_tags(source in inputs()) {
        let in_tokens: Vec<jinja::Tag> = tokenize(&source, Mode::Jinja)
            .iter()
            .flat_map(|t| {
                jinja::tags(t.text(&source)).map(move |tag| jinja::Tag {
                    range: tag.range.start + t.start()..tag.range.end + t.start(),
                    ..tag
                })
            })
            .collect();
        let in_source: Vec<jinja::Tag> = jinja::tags(&source).collect();
        prop_assert_eq!(in_tokens, in_source);
    }

    /// Text that cannot contain a tag lexes the same in both modes.
    #[test]
    fn jinja_mode_without_tags_is_sql(source in inputs().prop_map(|s| s.replace('{', "("))) {
        prop_assert_eq!(tokenize(&source, Mode::Jinja), tokenize(&source, Mode::Sql));
    }

    /// The verifier accepts what a printer that follows [`can_touch`]
    /// produces. Inline `INSERT` data is left out: its whitespace is data,
    /// and squashing it is a change the verifier must reject.
    #[test]
    fn verify_accepts_what_can_touch_allows(source in prop_oneof![statement_inputs(), inputs()]) {
        let mode = Mode::Sql;
        let error = tokenize(&source, mode).into_iter().find(|t| t.kind().is_error());
        let source = &source[..error.map_or(source.len(), Token::start)];
        let has_data = |text: &str| {
            split(text, mode).tokens().iter().any(|t| t.kind() == TokenKind::InsertData)
        };
        prop_assume!(!has_data(source));
        let tokens = tokenize(source, mode);
        prop_assume!(!tokens.iter().any(|t| t.kind().is_error()));
        let squashed = squash(source, &tokens, mode);
        prop_assume!(!has_data(&squashed));
        let result = verify(source, &squashed, mode, &Facts::new());
        prop_assert!(result.is_ok(), "{:?} squashed to {:?}: {:?}", source, squashed, result);
    }

    /// The splitter's tokens keep the lexer's guarantees, and its statements
    /// are ordered runs of tokens with only trivia between them.
    #[test]
    fn statements_partition_the_tokens(
        source in prop_oneof![statement_inputs(), inputs()],
        mode in modes(),
    ) {
        let split = split(&source, mode);
        let tokens = split.tokens();
        check_lossless(&source, tokens)?;
        for pair in tokens.windows(2) {
            prop_assert!(
                !(pair[0].kind() == TokenKind::Whitespace && pair[1].kind() == TokenKind::Whitespace),
                "adjacent whitespace {:?}", pair
            );
        }
        let mut after = 0;
        for statement in split.statements() {
            let range = statement.tokens.clone();
            prop_assert!(range.start >= after && range.start < range.end);
            prop_assert!(tokens[after..range.start].iter().all(|t| t.kind().is_trivia()));
            prop_assert!(tokens[range.start].kind().is_significant());
            prop_assert!(tokens[range.end - 1].kind().is_significant());
            if let Some(data) = statement.data {
                prop_assert!(range.contains(&data.token));
                prop_assert_eq!(tokens[data.token].kind(), TokenKind::InsertData);
            }
            after = range.end;
        }
        prop_assert!(tokens[after..].iter().all(|t| t.kind().is_trivia()));
    }
}
