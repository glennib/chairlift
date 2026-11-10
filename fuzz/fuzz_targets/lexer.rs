//! The lexer's guarantees on arbitrary input, in both modes: no panic, tokens
//! non-empty, contiguous and covering the input, tags never split, and a
//! printer that follows `can_touch` reproduces the tokens.

#![no_main]

use chairlift::lexer::Mode;
use chairlift::lexer::Token;
use chairlift::lexer::TokenKind;
use chairlift::lexer::can_touch;
use chairlift::lexer::jinja;
use chairlift::lexer::tokenize;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    for mode in [Mode::Sql, Mode::Jinja] {
        let tokens = tokenize(source, mode);
        let mut pos = 0;
        for token in &tokens {
            assert_eq!(token.start(), pos, "{token:?} starts after a gap");
            assert!(token.end() > token.start(), "{token:?} is empty");
            let _ = token.text(source);
            pos = token.end();
        }
        assert_eq!(pos, source.len());
        check_can_touch(source, &tokens, mode);
    }
    check_tags(source);
});

/// Drops the whitespace before the first error token and re-joins the tokens
/// as a printer would; the re-lexed tokens must be the same.
fn check_can_touch(source: &str, tokens: &[Token], mode: Mode) {
    let end = tokens
        .iter()
        .find(|t| t.kind().is_error())
        .map_or(source.len(), |t| t.start());
    let source = &source[..end];
    let tokens = tokenize(source, mode);
    if tokens.iter().any(|t| t.kind().is_error()) {
        return;
    }
    let is_space = |t: &Token| t.kind() == TokenKind::Whitespace;
    let mut out = String::new();
    // ClickHouse lexes the very first and last bytes of the input specially,
    // so leading and trailing whitespace each stay as one space.
    if tokens.first().is_some_and(is_space) {
        out.push(' ');
    }
    let mut prev_significant = None;
    let mut last: Option<Token> = None;
    for &token in tokens.iter().filter(|t| !is_space(t)) {
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
    if tokens.len() > 1 && tokens.last().is_some_and(is_space) {
        let after_comment = last.is_some_and(|t| t.kind() == TokenKind::LineComment);
        out.push(if after_comment { '\n' } else { ' ' });
    }

    let significant = |text: &str, tokens: &[Token]| -> Vec<(TokenKind, String)> {
        tokens
            .iter()
            .filter(|t| !is_space(t))
            .map(|t| (t.kind(), t.text(text).to_owned()))
            .collect()
    };
    assert_eq!(
        significant(&out, &tokenize(&out, mode)),
        significant(source, &tokens),
        "printed as {out:?}"
    );
}

/// The tags inside the Jinja-mode tokens are the tags in the whole text.
fn check_tags(source: &str) {
    let in_tokens: Vec<_> = tokenize(source, Mode::Jinja)
        .iter()
        .flat_map(|t| {
            jinja::tags(t.text(source)).map(move |tag| {
                (
                    tag.kind,
                    tag.range.start + t.start()..tag.range.end + t.start(),
                )
            })
        })
        .collect();
    let in_source: Vec<_> = jinja::tags(source).map(|t| (t.kind, t.range)).collect();
    assert_eq!(in_tokens, in_source);
}
