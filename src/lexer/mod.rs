//! The lossless lexer.
//!
//! [`tokenize`] splits a source text into [`Token`]s that cover it exactly:
//!
//! - every byte belongs to exactly one token, and the tokens appear in source
//!   order, each starting where the previous one ended, so concatenating their
//!   texts gives back the input;
//! - no token is empty;
//! - token boundaries fall on `char` boundaries, so [`Token::text`] never
//!   panics on the source the tokens came from.
//!
//! Whitespace and comments are tokens like any other; [`TokenKind::is_trivia`]
//! tells them apart. A token is a kind and a byte range, so a token stream
//! costs 12 bytes per token and holds no copies of the text.
//!
//! The rules are ClickHouse's own (`src/Parsers/Lexer.cpp`) for the release in
//! [`CLICKHOUSE_VERSION`](crate::CLICKHOUSE_VERSION): lexing ClickHouse SQL
//! with chairlift gives the same tokens as ClickHouse, except that runs of
//! whitespace are always one token and comments are split into line and
//! block comments. Two of those rules depend on the previous significant
//! token, which is why the lexer is a stateful iterator:
//!
//! - `.` followed by a digit starts a number (`.5`), unless the previous
//!   significant token is `)`, `]`, a bare word, a quoted identifier or a
//!   number; then it is a [`Dot`](TokenKind::Dot), so `t.1` is tuple access.
//!   At the very start of the input, `.` always starts a number, even `.x`.
//! - after a `Dot`, a number is only digits and `_`, so `x.1.2` is
//!   `x . 1 . 2`.
//!
//! Lexing never fails. Malformed input, such as an unclosed string or comment,
//! becomes an [error token](TokenKind::is_error); unclosed constructs run to
//! the end of the input.

mod kind;
mod scan;

use std::fmt;
use std::iter::FusedIterator;
use std::ops::Range;

pub use kind::TokenKind;

use crate::keyword::Keyword;

/// A token: a kind and a byte range of the source text.
///
/// Tokens do not borrow the source; [`text`](Self::text) slices it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Token {
    start: u32,
    end: u32,
    kind: TokenKind,
    keyword: Option<Keyword>,
}

impl Token {
    /// The token's kind.
    #[must_use]
    pub const fn kind(self) -> TokenKind {
        self.kind
    }

    /// For a [`BareWord`](TokenKind::BareWord) whose text is one of
    /// ClickHouse's keyword words (case-insensitively), that keyword;
    /// otherwise `None`.
    ///
    /// This classifies the text only: whether the word is used as a keyword or
    /// as an identifier is for the parser to decide.
    #[must_use]
    pub const fn keyword(self) -> Option<Keyword> {
        self.keyword
    }

    /// The byte offset of the token's first byte.
    #[must_use]
    pub const fn start(self) -> usize {
        self.start as usize
    }

    /// The byte offset just past the token's last byte.
    #[must_use]
    pub const fn end(self) -> usize {
        self.end as usize
    }

    /// The token's byte range in the source.
    #[must_use]
    pub const fn range(self) -> Range<usize> {
        self.start()..self.end()
    }

    /// The token's text.
    ///
    /// # Panics
    ///
    /// If `source` is shorter than the token's range or the range does not fall
    /// on `char` boundaries of `source`, which cannot happen for the source the
    /// token was lexed from.
    #[must_use]
    pub fn text(self, source: &str) -> &str {
        &source[self.range()]
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}@{}..{}", self.kind, self.start, self.end)?;
        if let Some(keyword) = self.keyword {
            write!(f, "({keyword:?})")?;
        }
        Ok(())
    }
}

/// Lexes `source` into tokens; see the [module documentation](self) for what
/// the tokens guarantee.
///
/// ```
/// use chairlift::lexer::{TokenKind, tokenize};
///
/// let source = "SELECT 1--1";
/// let kinds: Vec<TokenKind> = tokenize(source).iter().map(|t| t.kind()).collect();
/// assert_eq!(
///     kinds,
///     [TokenKind::BareWord, TokenKind::Whitespace, TokenKind::Number, TokenKind::LineComment]
/// );
/// ```
///
/// # Panics
///
/// If `source` is 4 GiB or longer; token offsets are 32 bits.
#[must_use]
pub fn tokenize(source: &str) -> Vec<Token> {
    let mut tokens = Vec::with_capacity(source.len() / 4 + 1);
    tokens.extend(Lexer::new(source));
    tokens
}

/// An iterator over the tokens of a source text; [`tokenize`] collects it.
#[derive(Clone, Debug)]
pub struct Lexer<'s> {
    scanner: scan::Scanner<'s>,
}

impl<'s> Lexer<'s> {
    /// A lexer over `source`.
    ///
    /// # Panics
    ///
    /// If `source` is 4 GiB or longer; token offsets are 32 bits.
    #[must_use]
    pub fn new(source: &'s str) -> Self {
        assert!(
            u32::try_from(source.len()).is_ok(),
            "chairlift lexes inputs up to 4 GiB"
        );
        Self {
            scanner: scan::Scanner::new(source, None, false),
        }
    }
}

impl Iterator for Lexer<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        self.scanner.next_token()
    }
}

impl FusedIterator for Lexer<'_> {}

/// Whether two tokens may be printed with nothing between them: whether
/// their concatenation lexes back to exactly the same two tokens.
///
/// `left` and `right` are each a token's kind and text. `prev_significant` is
/// the kind of the last [significant](TokenKind::is_significant) token before
/// `left`, or `None` at the start of a statement; the lexer needs it because
/// `.` and digits lex differently after some tokens.
///
/// This is what keeps a printer from turning `-` `-1` into a comment or `x`
/// `'0'` into a hex string. Beyond the pair, one ClickHouse rule looks further
/// ahead: a number that ends in an exponent marker (`1e`) followed by `+` or
/// `-` could absorb the sign and the digits after it, so that pair never
/// touches.
///
/// ```
/// use chairlift::lexer::{TokenKind, can_touch};
///
/// let minus = (TokenKind::Minus, "-");
/// assert!(can_touch(None, (TokenKind::BareWord, "a"), minus));
/// assert!(!can_touch(None, minus, minus));
/// assert!(can_touch(Some(TokenKind::BareWord), (TokenKind::Dot, "."), (TokenKind::BareWord, "b")));
/// ```
#[must_use]
pub fn can_touch(
    prev_significant: Option<TokenKind>,
    left: (TokenKind, &str),
    right: (TokenKind, &str),
) -> bool {
    let (left_kind, left_text) = left;
    let (right_kind, right_text) = right;
    if left_kind == TokenKind::Number
        && left_text.ends_with(['e', 'E', 'p', 'P'])
        && right_text.starts_with(['+', '-'])
    {
        return false;
    }
    let joined = [left_text, right_text].concat();
    let mut scanner = scan::Scanner::new(&joined, prev_significant, true);
    let first = scanner.next_token();
    let second = scanner.next_token();
    let third = scanner.next_token();
    matches!(
        (first, second, third),
        (Some(a), Some(b), None)
            if a.kind == left_kind
                && a.end() == left_text.len()
                && b.kind == right_kind
    )
}

#[cfg(test)]
mod tests;
