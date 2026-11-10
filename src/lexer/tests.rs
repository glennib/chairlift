use std::fmt::Write as _;

use super::*;

fn kinds(source: &str) -> Vec<TokenKind> {
    tokenize(source, Mode::Sql)
        .iter()
        .map(|t| t.kind())
        .collect()
}

#[test]
fn token_is_twelve_bytes() {
    assert_eq!(size_of::<Token>(), 12);
}

#[test]
fn empty_input_has_no_tokens() {
    assert_eq!(tokenize("", Mode::Sql), []);
}

#[test]
fn bare_words_carry_their_keyword() {
    let source = "select Settings, events";
    let keywords: Vec<Option<Keyword>> = tokenize(source, Mode::Sql)
        .iter()
        .filter(|t| t.kind() == TokenKind::BareWord)
        .map(|t| t.keyword())
        .collect();
    assert_eq!(
        keywords,
        [Some(Keyword::Select), Some(Keyword::Settings), None]
    );
}

#[test]
fn quoted_keywords_are_not_keywords() {
    let tokens = tokenize("`select` 'select' \"select\"", Mode::Sql);
    assert!(tokens.iter().all(|t| t.keyword().is_none()));
}

#[test]
fn debug_shows_kind_range_and_keyword() {
    let tokens = tokenize("from x", Mode::Sql);
    assert_eq!(
        format!("{tokens:?}"),
        "[BareWord@0..4(From), Whitespace@4..5, BareWord@5..6]"
    );
}

#[test]
fn dot_at_the_very_start_is_a_number() {
    assert_eq!(kinds(".x"), [TokenKind::Number, TokenKind::BareWord]);
    assert_eq!(
        kinds(" .x"),
        [TokenKind::Whitespace, TokenKind::Dot, TokenKind::BareWord]
    );
}

#[test]
fn touching_tokens() {
    use TokenKind::*;
    let word = |t| (BareWord, t);
    let cases = [
        // (prev, left, right, may touch)
        (None, word("a"), (Comma, ","), true),
        (None, word("a"), word("b"), false),
        (None, (Minus, "-"), (Number, "1"), true),
        (None, (Minus, "-"), (Minus, "-"), false),
        (None, (Minus, "-"), (Greater, ">"), false),
        (None, word("x"), (StringLiteral, "'0'"), false),
        (None, word("y"), (StringLiteral, "'0'"), true),
        (None, (Number, "1"), (Dot, "."), false),
        (Some(Dot), (Number, "1"), (Dot, "."), true),
        (Some(BareWord), (Dot, "."), (Number, "1"), true),
        (None, (Number, "1"), (Number, ".5"), false),
        (None, (Comma, ","), (Number, ".5"), true),
        (None, (Number, "1e"), (Plus, "+"), false),
        (None, (Number, "1"), (Plus, "+"), true),
        (None, (LineComment, "-- c"), word("x"), false),
        (None, (BlockComment, "/* c */"), word("x"), true),
        (None, (Less, "<"), (Equals, "="), false),
        (None, (Error, "#"), word("x"), true),
        (None, (Error, "#"), (Whitespace, " "), false),
        (None, (DollarSign, "$"), word("a"), false),
        (None, (DollarSign, "$"), (DollarSign, "$"), false),
        (None, (HereDoc, "$$a$$"), (DollarSign, "$"), false),
        (Some(BareWord), (HereDoc, "$$$$"), (Number, "6"), false),
        (None, (HereDoc, "$$a$$"), word("_b"), false),
        (None, (HereDoc, "$$a$$"), (Comma, ","), true),
        (None, (Comma, ","), (Number, "1e"), true),
        (None, (Colon, ":"), (Colon, ":"), false),
    ];
    for (prev, left, right, expected) in cases {
        assert_eq!(
            can_touch(Mode::Sql, prev, left, right),
            expected,
            "{prev:?} {left:?} {right:?}"
        );
    }
}

#[test]
fn glued_words_are_not_keywords() {
    let tokens = tokenize("select{{ x }}", Mode::Jinja);
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind(), TokenKind::BareWord);
    assert_eq!(tokens[0].keyword(), None);
}

#[test]
fn touching_tokens_in_jinja_mode() {
    use TokenKind::*;
    let cases = [
        ((JinjaExpr, "{{ a }}"), (BareWord, "b"), false),
        ((BareWord, "b"), (JinjaBlock, "{% if x %}"), false),
        ((JinjaExpr, "{{ a }}"), (JinjaExpr, "{{ b }}"), false),
        (
            (JinjaBlock, "{% if x %}"),
            (JinjaBlock, "{% endif %}"),
            true,
        ),
        ((JinjaExpr, "{{ a }}"), (Comma, ","), true),
        ((Dot, "."), (JinjaExpr, "{{ c }}"), true),
        ((OpeningCurlyBrace, "{"), (OpeningCurlyBrace, "{"), false),
    ];
    for (left, right, expected) in cases {
        assert_eq!(
            can_touch(Mode::Jinja, Some(BareWord), left, right),
            expected,
            "{left:?} {right:?}"
        );
    }
    assert!(can_touch(
        Mode::Sql,
        None,
        (OpeningCurlyBrace, "{"),
        (OpeningCurlyBrace, "{")
    ));
}

/// Distinct heredoc openers without closers each search the rest of the input
/// for theirs; the search stays linear in the input, not quadratic.
#[test]
fn heredoc_closer_search_is_linear() {
    for mode in [Mode::Sql, Mode::Jinja] {
        let mut source = String::new();
        for i in 0..20_000 {
            write!(source, "$t{i}$ {{{{ x }}}} ").unwrap();
        }
        let mut scanner = scan::Scanner::new(&source, mode, None, false);
        let mut words = 0;
        while let Some(token) = scanner.next_token() {
            words += usize::from(
                token.kind() == TokenKind::BareWord && token.text(&source).starts_with("$t"),
            );
        }
        assert_eq!(words, 20_000);
        assert!(
            scanner.closer_steps <= 2 * source.len(),
            "{mode:?}: {} steps for {} bytes",
            scanner.closer_steps,
            source.len()
        );
    }
}

#[test]
fn is_whitespace_agrees_with_the_lexer() {
    for c in (0..=0x3100)
        .chain(0xfe00..=0xffff)
        .filter_map(char::from_u32)
    {
        let text = c.to_string();
        let lexed = matches!(
            tokenize(&text, Mode::Sql)[..],
            [token] if token.kind() == TokenKind::Whitespace
        );
        assert_eq!(is_whitespace(c), lexed, "{c:?}");
    }
}
