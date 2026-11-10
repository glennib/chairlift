//! The byte-level rules, transcribed from ClickHouse's `src/Parsers/Lexer.cpp`.
//!
//! The bounds checks (`pos + 1 < end` and the like) are ClickHouse's and are
//! kept exactly, because they decide edge cases at the end of the input: `1e`
//! at the end is a bare word, but `1e+` is the number `1e` and a `+`.

use super::Token;
use super::TokenKind;
use crate::keyword::Keyword;

#[derive(Clone, Debug)]
pub(super) struct Scanner<'s> {
    source: &'s str,
    bytes: &'s [u8],
    pos: usize,
    /// The kind of the last significant token, for the `.` and digit rules.
    prev: Option<TokenKind>,
    /// The input continues text that came before it, so its first byte is not
    /// the start of a query (where ClickHouse always reads `.` as a number).
    continued: bool,
}

impl<'s> Scanner<'s> {
    pub(super) fn new(source: &'s str, prev: Option<TokenKind>, continued: bool) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            pos: 0,
            prev,
            continued,
        }
    }

    pub(super) fn next_token(&mut self) -> Option<Token> {
        let start = self.pos;
        if start >= self.bytes.len() {
            return None;
        }
        let kind = self.sql_token(start);
        debug_assert!(self.pos > start);
        let keyword = if kind == TokenKind::BareWord {
            Keyword::from_word(&self.source[start..self.pos])
        } else {
            None
        };
        if kind.is_significant() {
            self.prev = Some(kind);
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "`Lexer::new` rejects inputs of 4 GiB or more"
        )]
        let token = Token {
            start: start as u32,
            end: self.pos as u32,
            kind,
            keyword,
        };
        Some(token)
    }

    /// Whether the previous significant token can be followed by tuple access,
    /// which makes `.1` a `Dot` and a number rather than the number `.1`.
    fn after_operand(&self) -> bool {
        matches!(
            self.prev,
            Some(
                TokenKind::ClosingRoundBracket
                    | TokenKind::ClosingSquareBracket
                    | TokenKind::BareWord
                    | TokenKind::QuotedIdentifier
                    | TokenKind::Number
            )
        )
    }

    /// Lexes one token starting at `start`, leaving `self.pos` at its end.
    fn sql_token(&mut self, start: usize) -> TokenKind {
        let b = self.bytes;
        let end = b.len();
        let single = |this: &mut Self, kind| {
            this.pos = start + 1;
            kind
        };
        let pair = |this: &mut Self, second: u8, two, one| {
            if start + 1 < end && b[start + 1] == second {
                this.pos = start + 2;
                two
            } else {
                this.pos = start + 1;
                one
            }
        };
        match b[start] {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0c | 0x0b => {
                self.pos = skip_whitespace(b, start);
                TokenKind::Whitespace
            }
            b'0'..=b'9' => self.number(start),
            b'\'' => self.quoted(
                start,
                b'\'',
                TokenKind::StringLiteral,
                TokenKind::ErrorSingleQuoteIsNotClosed,
            ),
            b'"' => self.quoted(
                start,
                b'"',
                TokenKind::QuotedIdentifier,
                TokenKind::ErrorDoubleQuoteIsNotClosed,
            ),
            b'`' => self.quoted(
                start,
                b'`',
                TokenKind::QuotedIdentifier,
                TokenKind::ErrorBackQuoteIsNotClosed,
            ),
            b'(' => single(self, TokenKind::OpeningRoundBracket),
            b')' => single(self, TokenKind::ClosingRoundBracket),
            b'[' => single(self, TokenKind::OpeningSquareBracket),
            b']' => single(self, TokenKind::ClosingSquareBracket),
            b'{' => single(self, TokenKind::OpeningCurlyBrace),
            b'}' => single(self, TokenKind::ClosingCurlyBrace),
            b',' => single(self, TokenKind::Comma),
            b';' => single(self, TokenKind::Semicolon),
            b'.' => self.dot(start),
            b'+' => single(self, TokenKind::Plus),
            b'-' => {
                if start + 1 < end && b[start + 1] == b'>' {
                    self.pos = start + 2;
                    TokenKind::Arrow
                } else if start + 1 < end && b[start + 1] == b'-' {
                    self.line_comment(start + 2)
                } else {
                    self.pos = start + 1;
                    TokenKind::Minus
                }
            }
            b'*' => single(self, TokenKind::Asterisk),
            b'/' => {
                if start + 1 < end && b[start + 1] == b'/' {
                    self.line_comment(start + 2)
                } else if start + 1 < end && b[start + 1] == b'*' {
                    self.block_comment(start + 2)
                } else {
                    self.pos = start + 1;
                    TokenKind::Slash
                }
            }
            b'#' => {
                if start + 1 < end && matches!(b[start + 1], b' ' | b'!') {
                    self.line_comment(start + 1)
                } else {
                    self.pos = start + 1;
                    TokenKind::Error
                }
            }
            b'%' => single(self, TokenKind::Percent),
            b'=' => pair(self, b'=', TokenKind::Equals, TokenKind::Equals),
            b'!' => {
                if start + 1 < end && b[start + 1] == b'=' {
                    self.pos = start + 2;
                    TokenKind::NotEquals
                } else if start + 1 < end && b[start + 1] == b'~' {
                    pair_at(
                        self,
                        start + 1,
                        b'*',
                        TokenKind::NotTildeAsterisk,
                        TokenKind::NotTilde,
                    )
                } else {
                    self.pos = start + 1;
                    TokenKind::ErrorSingleExclamationMark
                }
            }
            b'~' => pair(self, b'*', TokenKind::TildeAsterisk, TokenKind::Tilde),
            b'<' => {
                if start + 2 < end && b[start + 1] == b'=' && b[start + 2] == b'>' {
                    self.pos = start + 3;
                    TokenKind::Spaceship
                } else if start + 1 < end && b[start + 1] == b'=' {
                    self.pos = start + 2;
                    TokenKind::LessOrEquals
                } else if start + 1 < end && b[start + 1] == b'>' {
                    self.pos = start + 2;
                    TokenKind::NotEquals
                } else {
                    self.pos = start + 1;
                    TokenKind::Less
                }
            }
            b'>' => pair(self, b'=', TokenKind::GreaterOrEquals, TokenKind::Greater),
            b'?' => single(self, TokenKind::QuestionMark),
            b'^' => single(self, TokenKind::Caret),
            b':' => pair(self, b':', TokenKind::DoubleColon, TokenKind::Colon),
            b'|' => {
                if start + 1 < end && b[start + 1] == b'|' {
                    self.pos = start + 2;
                    TokenKind::Concatenation
                } else if start + 1 < end && b[start + 1] == b'>' {
                    self.pos = start + 2;
                    TokenKind::PipeOperator
                } else {
                    self.pos = start + 1;
                    TokenKind::PipeMark
                }
            }
            b'@' => pair(self, b'@', TokenKind::DoubleAt, TokenKind::At),
            b'\\' => pair(self, b'G', TokenKind::VerticalDelimiter, TokenKind::Error),
            0xe2 if start + 3 <= end && b[start + 1] == 0x88 && b[start + 2] == 0x92 => {
                // U+2212 MINUS SIGN
                self.pos = start + 3;
                TokenKind::Minus
            }
            0xe2 if start + 5 < end
                && b[start + 1] == 0x80
                && matches!(b[start + 2], 0x98 | 0x9c) =>
            {
                // ‘…’ (U+2018, U+2019) or “…” (U+201C, U+201D)
                if b[start + 2] == 0x98 {
                    self.unicode_quoted(
                        start,
                        0x99,
                        TokenKind::StringLiteral,
                        TokenKind::ErrorSingleQuoteIsNotClosed,
                    )
                } else {
                    self.unicode_quoted(
                        start,
                        0x9d,
                        TokenKind::QuotedIdentifier,
                        TokenKind::ErrorDoubleQuoteIsNotClosed,
                    )
                }
            }
            _ => self.other(start),
        }
    }

    /// ClickHouse's `default:` branch: heredocs, `$`, hex and binary strings,
    /// bare words, Unicode whitespace and unrecognised characters.
    fn other(&mut self, start: usize) -> TokenKind {
        let b = self.bytes;
        let end = b.len();
        if b[start] == b'$' {
            if let Some(kind) = self.heredoc(start) {
                return kind;
            }
            if start + 1 == end || !is_word_char(b[start + 1]) {
                self.pos = start + 1;
                return TokenKind::DollarSign;
            }
        }
        if start + 2 < end && b[start + 1] == b'\'' && matches!(b[start], b'x' | b'X' | b'b' | b'B')
        {
            return self.hex_or_bin_string(start);
        }
        if is_word_char(b[start]) || b[start] == b'$' {
            let mut pos = start + 1;
            while pos < end && (is_word_char(b[pos]) || b[pos] == b'$') {
                pos += 1;
            }
            self.pos = pos;
            return TokenKind::BareWord;
        }
        let pos = skip_whitespace(b, start);
        if pos > start {
            self.pos = pos;
            return TokenKind::Whitespace;
        }
        let mut pos = start + 1;
        while pos < end && is_continuation_byte(b[pos]) {
            pos += 1;
        }
        self.pos = pos;
        TokenKind::Error
    }

    /// `$tag$…$tag$`, if a heredoc starts at `start`.
    fn heredoc(&mut self, start: usize) -> Option<TokenKind> {
        let b = self.bytes;
        let tag_end = start + 1 + b[start + 1..].iter().position(|&c| c == b'$')?;
        if !b[start + 1..tag_end].iter().all(|&c| is_word_char(c)) {
            return None;
        }
        let opener = &b[start..=tag_end];
        let body = tag_end + 1;
        let close = b[body..]
            .windows(opener.len())
            .position(|window| window == opener)?;
        self.pos = body + close + opener.len();
        Some(TokenKind::HereDoc)
    }

    fn hex_or_bin_string(&mut self, start: usize) -> TokenKind {
        let b = self.bytes;
        let hex = matches!(b[start], b'x' | b'X');
        let mut pos = start + 2;
        while pos < b.len()
            && if hex {
                b[pos].is_ascii_hexdigit()
            } else {
                matches!(b[pos], b'0' | b'1')
            }
        {
            pos += 1;
        }
        if pos < b.len() && b[pos] == b'\'' {
            self.pos = pos + 1;
            TokenKind::StringLiteral
        } else {
            self.pos = b.len();
            TokenKind::ErrorSingleQuoteIsNotClosed
        }
    }

    /// `'…'`, `"…"` or `` `…` ``: a doubled quote or a backslash escapes.
    fn quoted(&mut self, start: usize, quote: u8, ok: TokenKind, error: TokenKind) -> TokenKind {
        let b = self.bytes;
        let end = b.len();
        let mut pos = start + 1;
        loop {
            while pos < end && b[pos] != quote && b[pos] != b'\\' {
                pos += 1;
            }
            if pos >= end {
                self.pos = end;
                return error;
            }
            if b[pos] == quote {
                pos += 1;
                if pos < end && b[pos] == quote {
                    pos += 1;
                    continue;
                }
                self.pos = pos;
                return ok;
            }
            pos += 1;
            if pos >= end {
                self.pos = end;
                return error;
            }
            pos += 1;
        }
    }

    /// `‘…’` or `“…”`, without escapes. `close` is the last byte of the
    /// closing quote's UTF-8 encoding (`E2 80 xx`).
    fn unicode_quoted(
        &mut self,
        start: usize,
        close: u8,
        ok: TokenKind,
        error: TokenKind,
    ) -> TokenKind {
        let b = self.bytes;
        let end = b.len();
        let mut pos = start + 3;
        loop {
            while pos < end && b[pos] != 0xe2 {
                pos += 1;
            }
            if pos + 2 >= end {
                self.pos = end;
                return error;
            }
            if b[pos + 1] == 0x80 && b[pos + 2] == close {
                self.pos = pos + 3;
                return ok;
            }
            pos += 1;
        }
    }

    /// A line comment whose marker ends before `pos`; the newline is not part
    /// of it.
    fn line_comment(&mut self, pos: usize) -> TokenKind {
        let b = self.bytes;
        self.pos = pos
            + b[pos..]
                .iter()
                .position(|&c| c == b'\n')
                .unwrap_or(b.len() - pos);
        TokenKind::LineComment
    }

    /// A block comment whose `/*` ends before `pos`.
    fn block_comment(&mut self, mut pos: usize) -> TokenKind {
        let b = self.bytes;
        let mut depth = 1usize;
        while pos + 2 <= b.len() {
            if b[pos] == b'/' && b[pos + 1] == b'*' {
                pos += 2;
                depth += 1;
            } else if b[pos] == b'*' && b[pos + 1] == b'/' {
                pos += 2;
                depth -= 1;
                if depth == 0 {
                    self.pos = pos;
                    return TokenKind::BlockComment;
                }
            } else {
                pos += 1;
            }
        }
        self.pos = b.len();
        TokenKind::ErrorMultilineCommentIsNotClosed
    }

    /// `.`: a `Dot`, or a number such as `.5` or `.5e-3`.
    fn dot(&mut self, start: usize) -> TokenKind {
        let b = self.bytes;
        let end = b.len();
        let at_begin = start == 0 && !self.continued;
        if !at_begin
            && (!(start + 1 < end && b[start + 1].is_ascii_digit()) || self.after_operand())
        {
            self.pos = start + 1;
            return TokenKind::Dot;
        }
        let mut pos = digits(b, start + 1, true, false);
        if pos + 1 < end && matches!(b[pos], b'e' | b'E') {
            pos += 1;
            if pos + 1 < end && matches!(b[pos], b'-' | b'+') {
                pos += 1;
            }
            pos = digits(b, pos, true, false);
        }
        self.pos = pos;
        TokenKind::Number
    }

    /// A token starting with a digit: a number, or a bare word like `1abc`.
    fn number(&mut self, start: usize) -> TokenKind {
        let b = self.bytes;
        let end = b.len();
        let mut pos;
        if self.prev == Some(TokenKind::Dot) {
            // Tuple access: `x.1.2` is `x . 1 . 2`, so no fraction.
            pos = digits(b, start + 1, false, false);
        } else {
            let mut start_of_block = false;
            let mut hex = false;
            pos = start + 1;
            if start + 2 < end
                && b[start] == b'0'
                && matches!(b[start + 1], b'x' | b'X' | b'b' | b'B')
            {
                let valid = if matches!(b[start + 1], b'x' | b'X') {
                    hex = b[start + 2].is_ascii_hexdigit();
                    hex
                } else {
                    matches!(b[start + 2], b'0' | b'1')
                };
                if valid {
                    pos = start + 2;
                    start_of_block = true;
                }
            }
            pos = digits(b, pos, start_of_block, hex);
            if pos < end && b[pos] == b'.' {
                pos = digits(b, pos + 1, true, hex);
            }
            let exponent: &[u8] = if hex { b"pP" } else { b"eE" };
            if pos + 1 < end && exponent.contains(&b[pos]) {
                pos += 1;
                if pos + 1 < end && matches!(b[pos], b'-' | b'+') {
                    pos += 1;
                }
                pos = digits(b, pos, true, false);
            }
        }
        if pos < end && is_word_char(b[pos]) {
            while pos < end && is_word_char(b[pos]) {
                pos += 1;
            }
            self.pos = pos;
            if b[start..pos].iter().all(|&c| is_word_char(c) || c == b'$') {
                return TokenKind::BareWord;
            }
            return TokenKind::ErrorWrongNumber;
        }
        self.pos = pos;
        TokenKind::Number
    }
}

/// `pos` advanced over a run of digits (hex digits if `hex`) and `_`
/// separators. `start_of_block` is true where a separator may not appear yet,
/// as right after `0x` or the decimal point.
fn digits(b: &[u8], mut pos: usize, mut start_of_block: bool, hex: bool) -> usize {
    let is_digit = |c: u8| {
        if hex {
            c.is_ascii_hexdigit()
        } else {
            c.is_ascii_digit()
        }
    };
    while pos < b.len() && (is_digit(b[pos]) || is_number_separator(b, pos, start_of_block, hex)) {
        pos += 1;
        start_of_block = false;
    }
    pos
}

/// ClickHouse's `isNumberSeparator`: `_` between two digits.
fn is_number_separator(b: &[u8], pos: usize, start_of_block: bool, hex: bool) -> bool {
    if b[pos] != b'_' || start_of_block {
        return false;
    }
    match b.get(pos + 1) {
        Some(&next) => {
            if hex {
                next.is_ascii_hexdigit()
            } else {
                next.is_ascii_digit()
            }
        }
        None => false,
    }
}

/// ClickHouse's `isWordCharASCII`: `[A-Za-z0-9_]`.
fn is_word_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn is_continuation_byte(c: u8) -> bool {
    c & 0b1100_0000 == 0b1000_0000
}

/// ClickHouse's `skipWhitespacesUTF8`: `pos` advanced over ASCII and Unicode
/// whitespace.
fn skip_whitespace(b: &[u8], mut pos: usize) -> usize {
    let end = b.len();
    while pos < end {
        match b[pos] {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0c | 0x0b => pos += 1,
            // U+0085, U+00A0
            0xc2 if pos + 1 < end && matches!(b[pos + 1], 0x85 | 0xa0) => pos += 2,
            0xe1 | 0xe2 | 0xe3 | 0xef
                if pos + 2 < end && is_wide_space(b[pos], b[pos + 1], b[pos + 2]) =>
            {
                pos += 3;
            }
            _ => break,
        }
    }
    pos
}

/// The three-byte whitespace characters: U+180E, U+2000–U+200D, U+2028,
/// U+2029, U+202F, U+205F, U+2060, U+3000 and U+FEFF.
fn is_wide_space(a: u8, b: u8, c: u8) -> bool {
    match (a, b) {
        (0xe1, 0xa0) => c == 0x8e,
        (0xe2, 0x80) => matches!(c, 0x80..=0x8d | 0xa8 | 0xa9 | 0xaf),
        (0xe2, 0x81) => matches!(c, 0x9f | 0xa0),
        (0xe3, 0x80) => c == 0x80,
        (0xef, 0xbb) => c == 0xbf,
        _ => false,
    }
}

/// Like the `pair` closure in `sql_token`, for a token whose second byte is at
/// `at`.
fn pair_at(
    this: &mut Scanner<'_>,
    at: usize,
    third: u8,
    three: TokenKind,
    two: TokenKind,
) -> TokenKind {
    if at + 1 < this.bytes.len() && this.bytes[at + 1] == third {
        this.pos = at + 2;
        three
    } else {
        this.pos = at + 1;
        two
    }
}
