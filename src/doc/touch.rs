//! Whether two tokens may touch, quickly.

use crate::lexer::Mode;
use crate::lexer::TokenKind;
use crate::lexer::can_touch;

/// [`can_touch`], answered without lexing for the pairs a layout glues most:
/// a call's name and `(`, brackets and what they enclose, `,` and `;` after
/// an operand, `.` and `::` between names. Every other pair is lexed.
pub(super) fn may_touch(
    mode: Mode,
    prev_significant: Option<TokenKind>,
    left: (TokenKind, &str),
    right: (TokenKind, &str),
) -> bool {
    known_to_touch(left, right) || can_touch(mode, prev_significant, left, right)
}

/// Whether the pair is one that always lexes back as the same two tokens,
/// whatever came before it. Only fixed-text punctuation, plain words of ASCII
/// letters, digits and `_` (no `$`, no Jinja tag) that start with a letter
/// or `_`, unsigned integers, strings and quoted identifiers qualify; the
/// `tests` module checks every answer against [`can_touch`].
pub(super) fn known_to_touch(left: (TokenKind, &str), right: (TokenKind, &str)) -> bool {
    use TokenKind as K;
    let (lk, lt) = left;
    let (rk, rt) = right;
    let word = |k: K, t: &str| k == K::BareWord && is_plain_word(t);
    let integer =
        |k: K, t: &str| k == K::Number && !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    let quoted = |k: K| matches!(k, K::QuotedIdentifier | K::StringLiteral);
    let closer = |k: K| matches!(k, K::ClosingRoundBracket | K::ClosingSquareBracket);
    let opener = |k: K| matches!(k, K::OpeningRoundBracket | K::OpeningSquareBracket);
    match (lk, rk) {
        (_, K::Comma | K::Semicolon | K::ClosingRoundBracket | K::ClosingSquareBracket) => {
            word(lk, lt)
                || integer(lk, lt)
                || quoted(lk)
                || closer(lk)
                || opener(lk)
                || lk == K::Asterisk
        }
        (K::OpeningRoundBracket | K::OpeningSquareBracket, _) => {
            word(rk, rt) || integer(rk, rt) || quoted(rk) || opener(rk) || rk == K::Asterisk
        }
        (_, K::OpeningRoundBracket | K::OpeningSquareBracket) => {
            word(lk, lt) || lk == K::QuotedIdentifier || closer(lk)
        }
        (_, K::Dot) => word(lk, lt) || lk == K::QuotedIdentifier || closer(lk),
        (K::Dot, _) => word(rk, rt) || rk == K::QuotedIdentifier || rk == K::Asterisk,
        (_, K::DoubleColon) => word(lk, lt) || quoted(lk) || closer(lk),
        (K::DoubleColon, _) => word(rk, rt),
        _ => false,
    }
}

fn is_plain_word(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    /// Tokens of every kind the fast path names, and their neighbours that
    /// lex specially.
    const PIECES: &[&str] = &[
        "a",
        "_x",
        "a1",
        "e",
        "x",
        "b",
        "p",
        "E",
        "abc_9",
        "1abc",
        "a$",
        "$a",
        "$",
        "1",
        "0",
        "123",
        "1e",
        "1e5",
        "0x1F",
        "1.5",
        ".5",
        "1.",
        "1_000",
        "'s'",
        "x'41'",
        "b'01'",
        "''",
        "`q`",
        "\"q\"",
        "$$h$$",
        "(",
        ")",
        "[",
        "]",
        "{",
        "}",
        ",",
        ";",
        ".",
        "::",
        ":",
        "*",
        "/",
        "-",
        "+",
        "=",
        "<",
        ">",
        "!=",
        "|",
        "||",
        "#",
        "--",
        "/* c */",
        "-- c",
        "{{ x }}",
        "{% if %}",
        "{# c #}",
        "events_{{ s }}",
        "?",
        "@",
        "%",
        "^",
        "~",
        "->",
    ];

    #[test]
    fn known_pairs_can_touch() {
        let kinds = [
            None,
            Some(TokenKind::BareWord),
            Some(TokenKind::ClosingRoundBracket),
            Some(TokenKind::Number),
            Some(TokenKind::Dot),
            Some(TokenKind::Comma),
            Some(TokenKind::OpeningRoundBracket),
        ];
        let mut known = 0;
        for mode in [Mode::Sql, Mode::Jinja] {
            let tokens: Vec<(TokenKind, &str)> = PIECES
                .iter()
                .filter_map(|piece| match tokenize(piece, mode).as_slice() {
                    [token] => Some((token.kind(), *piece)),
                    _ => None,
                })
                .collect();
            for &left in &tokens {
                for &right in &tokens {
                    if !known_to_touch(left, right) {
                        continue;
                    }
                    known += 1;
                    for prev in kinds {
                        assert!(
                            can_touch(mode, prev, left, right),
                            "{left:?} {right:?} after {prev:?} in {mode:?}"
                        );
                    }
                }
            }
        }
        assert!(known > 300, "the fast path answers {known} pairs");
    }

    #[test]
    fn unknown_pairs_are_lexed() {
        let minus = (TokenKind::Minus, "-");
        assert!(!known_to_touch(minus, minus));
        assert!(!may_touch(Mode::Sql, None, minus, minus));
        let number = (TokenKind::Number, "1");
        assert!(may_touch(Mode::Sql, None, minus, number));
    }
}
