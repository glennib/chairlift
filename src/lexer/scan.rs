//! The byte-level rules, transcribed from ClickHouse's `src/Parsers/Lexer.cpp`.
//!
//! The bounds checks (`pos + 1 < end` and the like) are ClickHouse's and are
//! kept exactly, because they decide edge cases at the end of the input: `1e`
//! at the end is a bare word, but `1e+` is the number `1e` and a `+`.
//!
//! In Jinja mode, every loop that looks for the end of a string, quoted
//! identifier, heredoc or comment steps over whole tags ([`Scanner::tag_at`]),
//! so a quote or closer inside a tag never ends the SQL construct around it.

use std::collections::HashMap;

use super::Mode;
use super::Token;
use super::TokenKind;
use super::jinja;
use super::jinja::TagKind;
use crate::keyword::Keyword;

#[derive(Clone, Debug)]
pub(super) struct Scanner<'s> {
    source: &'s str,
    bytes: &'s [u8],
    pos: usize,
    jinja: bool,
    /// The kind of the last significant token, for the `.` and digit rules.
    prev: Option<TokenKind>,
    /// The input continues text that came before it, so its first byte is not
    /// the start of a query (where ClickHouse always reads `.` as a number).
    continued: bool,
    /// Where the current run of glued block and comment tags ends, if it has
    /// been scanned and found to have nothing to join: its tags are single
    /// tokens.
    separate_tags_until: usize,
    /// Every heredoc closer candidate, built the first time a heredoc opener
    /// finds no closer: searching the rest of the input once per opener is
    /// quadratic on a text of many unclosed openers.
    closers: Option<Closers<'s>>,
    /// Bytes visited looking for heredoc closers, for a test that the search
    /// stays linear.
    #[cfg(test)]
    pub(super) closer_steps: usize,
}

/// For each tag, the offsets of the `$tag$` sequences in the input outside
/// Jinja tags, in order.
type Closers<'s> = HashMap<&'s [u8], Vec<u32>>;

/// A run of glued pieces: tags, and the word characters between them.
struct Run {
    end: usize,
    pieces: usize,
    /// The end and kind of the first piece, when it is a tag.
    first_tag: Option<(usize, TagKind)>,
    word: bool,
    expr: bool,
    raw: bool,
}

impl<'s> Scanner<'s> {
    pub(super) fn new(
        source: &'s str,
        mode: Mode,
        prev: Option<TokenKind>,
        continued: bool,
    ) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            pos: 0,
            jinja: mode == Mode::Jinja,
            prev,
            continued,
            separate_tags_until: 0,
            closers: None,
            #[cfg(test)]
            closer_steps: 0,
        }
    }

    pub(super) fn next_token(&mut self) -> Option<Token> {
        let start = self.pos;
        if start >= self.bytes.len() {
            return None;
        }
        let kind = if self.jinja && jinja::is_tag_start(self.bytes, start) {
            self.tags(start)
        } else {
            let kind = self.sql_token(start);
            if self.jinja
                && matches!(
                    kind,
                    TokenKind::BareWord | TokenKind::Number | TokenKind::DollarSign
                )
                && self.pos < self.bytes.len()
                && jinja::is_tag_start(self.bytes, self.pos)
            {
                self.glued_to_word(kind)
            } else {
                kind
            }
        };
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
    ///
    /// A `{{ … }}` or raw block renders as text the lexer cannot see; it is
    /// taken to end like a bare word, as in `{{ table }}.1`.
    fn after_operand(&self) -> bool {
        matches!(
            self.prev,
            Some(
                TokenKind::ClosingRoundBracket
                    | TokenKind::ClosingSquareBracket
                    | TokenKind::BareWord
                    | TokenKind::QuotedIdentifier
                    | TokenKind::Number
                    | TokenKind::JinjaExpr
                    | TokenKind::Verbatim
            )
        )
    }

    /// In Jinja mode, the end of the tag that starts at `pos`, if one does (the
    /// end of the input if it is not closed).
    #[inline]
    fn tag_at(&self, pos: usize) -> Option<usize> {
        if self.jinja && pos < self.bytes.len() && jinja::is_tag_start(self.bytes, pos) {
            Some(jinja::scan_tag(self.bytes, pos).end)
        } else {
            None
        }
    }

    /// The token at a tag in Jinja mode, starting at `start`.
    ///
    /// Tags glued to each other and to word characters form a run. A run that
    /// contains anything that renders text (a word, a `{{ … }}` or a raw block)
    /// is one token, because whitespace inserted anywhere inside it could split
    /// the rendered text. A run of only block and comment tags renders nothing,
    /// so its tags are separate tokens.
    fn tags(&mut self, start: usize) -> TokenKind {
        let b = self.bytes;
        if start < self.separate_tags_until {
            let tag = jinja::scan_tag(b, start);
            self.pos = tag.end;
            return single_tag_kind(tag.kind);
        }
        let run = self.run(start, false);
        let Some((first_end, first_kind)) = run.first_tag else {
            // The first tag is not closed.
            self.pos = b.len();
            return if jinja::scan_tag(b, start).kind == TagKind::Raw {
                TokenKind::ErrorJinjaRawIsNotClosed
            } else {
                TokenKind::ErrorJinjaTagIsNotClosed
            };
        };
        if run.pieces > 1 && (run.word || run.expr || run.raw) {
            self.pos = run.end;
            return if run.word {
                TokenKind::BareWord
            } else if run.expr {
                TokenKind::JinjaExpr
            } else {
                TokenKind::Verbatim
            };
        }
        self.separate_tags_until = run.end;
        self.pos = first_end;
        single_tag_kind(first_kind)
    }

    /// A bare word, number or `$` that ends at `self.pos`, where a tag starts:
    /// the word and the run of tags glued to it are one bare word.
    fn glued_to_word(&mut self, kind: TokenKind) -> TokenKind {
        let run = self.run(self.pos, true);
        if run.pieces > 1 {
            self.pos = run.end;
            TokenKind::BareWord
        } else {
            kind
        }
    }

    /// Scans the run of glued tags and word characters from `start`. With
    /// `after_word`, a word ends at `start` and counts as the run's first piece.
    /// An unclosed tag ends the run before it.
    fn run(&self, start: usize, after_word: bool) -> Run {
        let b = self.bytes;
        let mut run = Run {
            end: start,
            pieces: usize::from(after_word),
            first_tag: None,
            word: after_word,
            expr: false,
            raw: false,
        };
        let mut last_was_tag = false;
        let mut pos = start;
        while pos < b.len() {
            if jinja::is_tag_start(b, pos) {
                let tag = jinja::scan_tag(b, pos);
                if !tag.closed {
                    break;
                }
                match tag.kind {
                    TagKind::Expr => run.expr = true,
                    TagKind::Raw => run.raw = true,
                    TagKind::Block | TagKind::Comment => {}
                }
                if run.pieces == 0 {
                    run.first_tag = Some((tag.end, tag.kind));
                }
                pos = tag.end;
                last_was_tag = true;
            } else if last_was_tag && is_bare_word_char(b[pos]) {
                while pos < b.len() && is_bare_word_char(b[pos]) {
                    pos += 1;
                }
                run.word = true;
                last_was_tag = false;
            } else {
                break;
            }
            run.pieces += 1;
            run.end = pos;
        }
        run
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
    ///
    /// ClickHouse takes the text up to the next `$` as the tag and rejects it
    /// unless it is all word characters; scanning word characters and
    /// expecting a `$` after them is the same test without searching ahead.
    fn heredoc(&mut self, start: usize) -> Option<TokenKind> {
        let b = self.bytes;
        let mut tag_end = start + 1;
        while tag_end < b.len() && is_word_char(b[tag_end]) {
            tag_end += 1;
        }
        if b.get(tag_end) != Some(&b'$') {
            return None;
        }
        let opener = &b[start..=tag_end];
        let from = tag_end + 1;
        let closer = if let Some(closers) = &self.closers {
            closers.get(&b[start + 1..tag_end]).and_then(|offsets| {
                let i = offsets.partition_point(|&p| (p as usize) < from);
                offsets.get(i).map(|&p| p as usize)
            })
        } else {
            let mut found = None;
            let mut pos = from;
            while pos + opener.len() <= b.len() {
                #[cfg(test)]
                {
                    self.closer_steps += 1;
                }
                if b[pos..].starts_with(opener) {
                    found = Some(pos);
                    break;
                }
                pos = self.tag_at(pos).unwrap_or(pos + 1);
            }
            if found.is_none() {
                self.closers = Some(self.find_closers());
            }
            found
        };
        let pos = closer?;
        self.pos = pos + opener.len();
        Some(TokenKind::HereDoc)
    }

    /// Every `$tag$` outside Jinja tags, found with the same steps the search
    /// in [`heredoc`](Self::heredoc) takes, so a lookup finds the closer that
    /// search would find. A `$` starts at most one: the word characters after
    /// it, if a `$` follows them.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "`Lexer::new` rejects inputs of 4 GiB or more"
    )]
    fn find_closers(&mut self) -> Closers<'s> {
        let b = self.bytes;
        let mut closers = Closers::new();
        let mut pos = 0;
        while pos < b.len() {
            #[cfg(test)]
            {
                self.closer_steps += 1;
            }
            if let Some(end) = self.tag_at(pos) {
                pos = end;
                continue;
            }
            if b[pos] == b'$' {
                let mut end = pos + 1;
                while end < b.len() && is_word_char(b[end]) {
                    end += 1;
                }
                if b.get(end) == Some(&b'$') {
                    closers
                        .entry(&b[pos + 1..end])
                        .or_default()
                        .push(pos as u32);
                }
            }
            pos += 1;
        }
        closers
    }

    fn hex_or_bin_string(&mut self, start: usize) -> TokenKind {
        let b = self.bytes;
        let hex = matches!(b[start], b'x' | b'X');
        let mut pos = start + 2;
        loop {
            while pos < b.len()
                && if hex {
                    b[pos].is_ascii_hexdigit()
                } else {
                    matches!(b[pos], b'0' | b'1')
                }
            {
                pos += 1;
            }
            match self.tag_at(pos) {
                Some(tag_end) => pos = tag_end,
                None => break,
            }
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
    ///
    /// A tag right after a backslash is still a tag (the renderer does not
    /// know SQL escapes), so the backslash escapes the tag's rendered text.
    fn quoted(&mut self, start: usize, quote: u8, ok: TokenKind, error: TokenKind) -> TokenKind {
        let b = self.bytes;
        let end = b.len();
        let brace = if self.jinja { b'{' } else { quote };
        let mut pos = start + 1;
        loop {
            while pos < end && b[pos] != quote && b[pos] != b'\\' && b[pos] != brace {
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
            if b[pos] == b'\\' {
                pos += 1;
                if pos >= end {
                    self.pos = end;
                    return error;
                }
                if self.tag_at(pos).is_none() {
                    pos += 1;
                }
                continue;
            }
            pos = self.tag_at(pos).unwrap_or(pos + 1);
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
        let brace = if self.jinja { b'{' } else { 0xe2 };
        let mut pos = start + 3;
        loop {
            while pos < end && b[pos] != 0xe2 && b[pos] != brace {
                pos += 1;
            }
            if pos < end && b[pos] == b'{' {
                pos = self.tag_at(pos).unwrap_or(pos + 1);
                continue;
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
    ///
    /// In Jinja mode a tag inside the comment is skipped whole, newlines
    /// included, since the renderer sees it as a tag. A tag that is not closed
    /// makes the template unrenderable, so the comment is then an
    /// [`ErrorJinjaTagIsNotClosed`](TokenKind::ErrorJinjaTagIsNotClosed)
    /// running to the end of the input, as an unclosed tag in a string or block
    /// comment makes those unclosed.
    fn line_comment(&mut self, mut pos: usize) -> TokenKind {
        let b = self.bytes;
        while pos < b.len() && b[pos] != b'\n' {
            if self.jinja && jinja::is_tag_start(b, pos) {
                let tag = jinja::scan_tag(b, pos);
                if !tag.closed {
                    self.pos = b.len();
                    return TokenKind::ErrorJinjaTagIsNotClosed;
                }
                pos = tag.end;
            } else {
                pos += 1;
            }
        }
        self.pos = pos;
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
                pos = self.tag_at(pos).unwrap_or(pos + 1);
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

/// A byte that continues a bare word: a word character or `$`.
fn is_bare_word_char(c: u8) -> bool {
    is_word_char(c) || c == b'$'
}

fn single_tag_kind(kind: TagKind) -> TokenKind {
    match kind {
        TagKind::Expr => TokenKind::JinjaExpr,
        TagKind::Block => TokenKind::JinjaBlock,
        TagKind::Comment => TokenKind::JinjaComment,
        TagKind::Raw => TokenKind::Verbatim,
    }
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
