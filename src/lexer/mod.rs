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
//! tells them apart from the *significant* tokens the parser consumes. A token
//! is a kind and a byte range, so a token stream costs 12 bytes per token and
//! holds no copies of the text.
//!
//! # SQL
//!
//! The rules are ClickHouse's own (`src/Parsers/Lexer.cpp`) for the release in
//! [`CLICKHOUSE_VERSION`](crate::CLICKHOUSE_VERSION): lexing in [`Mode::Sql`]
//! gives the same tokens as ClickHouse, except that runs of whitespace are
//! always one token and comments are split into line and block comments. Two
//! of those rules depend on the previous significant token, which is why the
//! lexer is a stateful iterator:
//!
//! - `.` followed by a digit starts a number (`.5`), unless the previous
//!   significant token is `)`, `]`, a bare word, a quoted identifier or a
//!   number; then it is a [`Dot`](TokenKind::Dot), so `t.1` is tuple access.
//!   At the very start of the input, `.` always starts a number, even `.x`.
//! - after a `Dot`, a number is only digits and `_`, so `x.1.2` is
//!   `x . 1 . 2`.
//!
//! # Jinja
//!
//! In [`Mode::Jinja`], Jinja tags are found first, wherever they are, as the
//! renderer finds them (see [`jinja`]), and SQL is lexed around them:
//!
//! - a tag on its own is a [`JinjaExpr`](TokenKind::JinjaExpr),
//!   [`JinjaBlock`](TokenKind::JinjaBlock),
//!   [`JinjaComment`](TokenKind::JinjaComment) or, for a raw block,
//!   [`Verbatim`](TokenKind::Verbatim);
//! - a tag inside a SQL string, quoted identifier, heredoc or comment is part
//!   of that token, and quotes or closers inside the tag do not end it:
//!   `'{{ "it's" }}'` is one string;
//! - tags glued to a bare word or number, or to each other, join into one
//!   token, since inserting whitespace there would split the rendered text;
//!   see [`TokenKind::BareWord`] and [`TokenKind::JinjaExpr`];
//! - for the `.` rule, a `{{ … }}` counts as a bare word and block and comment
//!   tags are skipped like whitespace.
//!
//! In [`Mode::Sql`], `{{` is two [`OpeningCurlyBrace`](TokenKind::OpeningCurlyBrace)
//! tokens.
//!
//! # Errors
//!
//! Lexing never fails. Malformed input, such as an unclosed string, comment or
//! tag, becomes an [error token](TokenKind::is_error); unclosed constructs run
//! to the end of the input.

pub mod jinja;
mod kind;
mod scan;

use std::fmt;
use std::iter::FusedIterator;
use std::ops::Range;
use std::path::Path;

pub use kind::TokenKind;

use crate::keyword::Keyword;

/// Whether the source is plain SQL or a Jinja template.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Plain ClickHouse SQL.
    #[default]
    Sql,
    /// SQL with Jinja tags, such as a chum `.sql.j2` migration or a dbt model.
    Jinja,
}

impl Mode {
    /// The mode for a file, by its name: [`Mode::Jinja`] for names ending in
    /// `.j2` (as `.sql.j2`) or `.j2.sql`, chum's two template extensions;
    /// otherwise [`Mode::Sql`].
    ///
    /// The name is only a default: dbt models, for one, are templates named
    /// `.sql`.
    ///
    /// ```
    /// use chairlift::lexer::Mode;
    /// use std::path::Path;
    ///
    /// assert_eq!(Mode::for_path(Path::new("0001_base.up.sql.j2")), Mode::Jinja);
    /// assert_eq!(Mode::for_path(Path::new("0003.up.j2.sql")), Mode::Jinja);
    /// assert_eq!(Mode::for_path(Path::new("schema.sql")), Mode::Sql);
    /// ```
    #[must_use]
    pub fn for_path(path: &Path) -> Mode {
        let name = path.file_name().map(|n| n.to_string_lossy());
        if name.is_some_and(|n| n.ends_with(".j2") || n.ends_with(".j2.sql")) {
            Mode::Jinja
        } else {
            Mode::Sql
        }
    }
}

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
    /// as an identifier is for the parser to decide. A word with Jinja tags
    /// glued into it is never a keyword.
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

    /// A token of a kind the lexer does not classify further, such as
    /// [`InsertData`](TokenKind::InsertData) or a piece of a whitespace run.
    ///
    /// `range` must lie within an input of under 4 GiB, as every token's does.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "token ranges lie within inputs `Lexer::new` accepted"
    )]
    pub(crate) fn synthetic(kind: TokenKind, range: Range<usize>) -> Token {
        debug_assert!(range.start < range.end && u32::try_from(range.end).is_ok());
        Token {
            start: range.start as u32,
            end: range.end as u32,
            kind,
            keyword: None,
        }
    }

    /// The token moved `by` bytes later, for tokens lexed from a suffix of a
    /// text.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the suffix and its offset lie within an input `Lexer::new` accepted"
    )]
    pub(crate) fn shifted(self, by: usize) -> Token {
        Token {
            start: self.start + by as u32,
            end: self.end + by as u32,
            ..self
        }
    }

    /// The token extended to end at `end`, for merging adjacent whitespace.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "token ranges lie within inputs `Lexer::new` accepted"
    )]
    pub(crate) fn extended_to(self, end: usize) -> Token {
        Token {
            end: end as u32,
            ..self
        }
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
/// use chairlift::lexer::{Mode, TokenKind, tokenize};
///
/// let kinds = |source, mode| -> Vec<TokenKind> {
///     tokenize(source, mode).iter().map(|t| t.kind()).collect()
/// };
/// assert_eq!(
///     kinds("SELECT 1--1", Mode::Sql),
///     [TokenKind::BareWord, TokenKind::Whitespace, TokenKind::Number, TokenKind::LineComment]
/// );
/// assert_eq!(
///     kinds("FROM events_{{ suffix }}", Mode::Jinja),
///     [TokenKind::BareWord, TokenKind::Whitespace, TokenKind::BareWord]
/// );
/// ```
///
/// # Panics
///
/// If `source` is 4 GiB or longer; token offsets are 32 bits.
#[must_use]
pub fn tokenize(source: &str, mode: Mode) -> Vec<Token> {
    let mut tokens = Vec::with_capacity(source.len() / 4 + 1);
    tokens.extend(Lexer::new(source, mode));
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
    pub fn new(source: &'s str, mode: Mode) -> Self {
        assert!(
            u32::try_from(source.len()).is_ok(),
            "chairlift lexes inputs up to 4 GiB"
        );
        Self {
            scanner: scan::Scanner::new(source, mode, None, false),
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

/// Whether `c` is whitespace to ClickHouse, as in a
/// [`Whitespace`](TokenKind::Whitespace) token.
pub(crate) fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c' | '\u{85}' | '\u{a0}' | '\u{180e}' | '\u{2000}'
            ..='\u{200d}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{2060}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// Whether two tokens may be printed with nothing between them: whether
/// their concatenation lexes back to exactly the same two tokens.
///
/// `left` and `right` are each a token's kind and text. `prev_significant` is
/// the kind of the last [significant](TokenKind::is_significant) token before
/// `left`, or `None` at the start of a statement; the lexer needs it because
/// `.` and digits lex differently after some tokens.
///
/// This is what keeps a printer from turning `-` `-1` into a comment, `x`
/// `'0'` into a hex string, or `{{ a }}` `b` into one rendered word. Two rules
/// reach beyond the pair, so those pairs never touch:
///
/// - a number that ends in an exponent marker (`1e`) followed by `+` or `-`
///   could absorb the sign and the digits after it;
/// - a `$` followed by a `$` makes a heredoc opener (`$$`) that a later `$$`
///   would close.
///
/// ```
/// use chairlift::lexer::{Mode, TokenKind, can_touch};
///
/// let minus = (TokenKind::Minus, "-");
/// assert!(can_touch(Mode::Sql, None, (TokenKind::BareWord, "a"), minus));
/// assert!(!can_touch(Mode::Sql, None, minus, minus));
/// let dot = (TokenKind::Dot, ".");
/// assert!(can_touch(Mode::Sql, Some(TokenKind::BareWord), dot, (TokenKind::BareWord, "b")));
/// ```
#[must_use]
pub fn can_touch(
    mode: Mode,
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
    if left_text.ends_with('$') && right_text.starts_with('$') {
        return false;
    }
    // A newline stands in for whatever follows `right`: some rules lex the
    // last bytes of the input differently (`1e` at the very end is a bare
    // word, elsewhere a number).
    let joined = [left_text, right_text, "\n"].concat();
    let mut scanner = scan::Scanner::new(&joined, mode, prev_significant, true);
    let first = scanner.next_token();
    let second = scanner.next_token();
    matches!(
        (first, second),
        (Some(a), Some(b))
            if a.kind == left_kind
                && a.end() == left_text.len()
                && b.kind == right_kind
                && b.end() == left_text.len() + right_text.len()
    )
}

/// Whether the tokens that end the input lex the same with a newline after
/// them. ClickHouse lexes the last bytes of the input specially (`1e` at the
/// very end is a bare word, elsewhere a number; `1e+` at the end is `1e` and
/// `+`, elsewhere one number), so adding a final newline can change the last
/// tokens.
///
/// `tail` is the end of the input from the start of a token a few tokens
/// before the end; `prev_significant` is as for [`can_touch`], and `continued`
/// says whether `tail` starts after the start of the input.
pub(crate) fn same_before_newline(
    mode: Mode,
    prev_significant: Option<TokenKind>,
    tail: &str,
    continued: bool,
) -> bool {
    let lex = |text: &str| -> Vec<(TokenKind, usize)> {
        let mut scanner = scan::Scanner::new(text, mode, prev_significant, continued);
        std::iter::from_fn(|| scanner.next_token())
            .map(|t| (t.kind, t.end()))
            .collect()
    };
    let mut joined = lex(&[tail, "\n"].concat());
    joined.pop_if(|&mut (kind, end)| kind == TokenKind::Whitespace && end == tail.len() + 1);
    joined == lex(tail)
}

#[cfg(test)]
mod tests;
