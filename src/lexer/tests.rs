use super::*;

fn kinds(source: &str) -> Vec<TokenKind> {
    tokenize(source).iter().map(|t| t.kind()).collect()
}

#[test]
fn token_is_twelve_bytes() {
    assert_eq!(size_of::<Token>(), 12);
}

#[test]
fn empty_input_has_no_tokens() {
    assert_eq!(tokenize(""), []);
}

#[test]
fn bare_words_carry_their_keyword() {
    let source = "select Settings, events";
    let keywords: Vec<Option<Keyword>> = tokenize(source)
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
    let tokens = tokenize("`select` 'select' \"select\"");
    assert!(tokens.iter().all(|t| t.keyword().is_none()));
}

#[test]
fn debug_shows_kind_range_and_keyword() {
    let tokens = tokenize("from x");
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
        (None, (Colon, ":"), (Colon, ":"), false),
    ];
    for (prev, left, right, expected) in cases {
        assert_eq!(
            can_touch(prev, left, right),
            expected,
            "{prev:?} {left:?} {right:?}"
        );
    }
}
