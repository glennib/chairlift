//! The parser's state: a cursor over the significant tokens of one
//! statement, and the events it writes.
//!
//! The grammar (see [`grammar`](super::grammar)) drives a [`Parser`]: it
//! looks at the current significant token, consumes it with one of the
//! `bump` methods, and opens and closes nodes with [`Marker`]s. Everything it
//! does is an [`Event`] in a list; [`build`](super::build) turns the list into
//! a tree once the whole text is parsed. Trivia never appears in events: the
//! builder places it (see [`syntax`](super#trivia)).
//!
//! A [`Checkpoint`] is the length of the event list and the token position;
//! [`Parser::rollback`] truncates the list back to it, which also drops every
//! keyword slot recorded after it, since keyword slots are token kinds in the
//! events.

use crate::keyword::Keyword;
use crate::lexer::Token;
use crate::lexer::TokenKind;
use crate::syntax::FallbackLevel;
use crate::syntax::SyntaxKind;

/// One step of the parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// A node starts. `kind` is `None` until its marker completes, and stays
    /// `None` if the marker is abandoned. `forward_parent` is the distance to
    /// the `Start` of a node that wraps this one (see
    /// [`CompletedMarker::precede`]), or 0.
    Start {
        kind: Option<SyntaxKind>,
        forward_parent: u32,
    },
    /// The innermost open node ends.
    Finish,
    /// The significant token at `index` in the text's tokens, as `kind`.
    Token { kind: SyntaxKind, index: u32 },
}

/// Why a region became verbatim, recorded by a fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fallback {
    /// How much was kept verbatim.
    pub level: FallbackLevel,
    /// The verbatim region, as a byte range of the text.
    pub range: std::ops::Range<usize>,
    /// What the parser expected where it gave up, and what it found:
    /// `expected ")", found keyword FROM`.
    pub message: String,
}

/// Where the parse failed furthest: what was expected at which token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Expected {
    pos: usize,
    what: &'static str,
}

/// A point the parser can return to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Checkpoint {
    events: usize,
    pos: usize,
    fallbacks: usize,
    preceded: usize,
    no_in: bool,
}

/// How deep grammar functions may nest, counting each expression, data type
/// and subquery level and each step of a chain of left-associative operators.
/// It bounds the parser's stack use and the depth of the tree; deeper input
/// fails, which a fallback turns into a verbatim region.
pub(crate) const MAX_DEPTH: u32 = 200;

pub(crate) struct Parser<'t> {
    source: &'t str,
    tokens: &'t [Token],
    /// Indices into `tokens` of the significant tokens of the whole text.
    significant: &'t [u32],
    /// The current position, an index into `significant`.
    pos: usize,
    /// The end of the current statement, an index into `significant`.
    end: usize,
    events: Vec<Event>,
    fallbacks: Vec<Fallback>,
    /// `Start` events whose `forward_parent` was set, so that a rollback can
    /// reset those that lie before its checkpoint.
    preceded: Vec<u32>,
    expected: Option<Expected>,
    depth: u32,
    /// Tokens that may still be re-read after rollbacks of speculative
    /// parses; see [`Parser::speculate`].
    budget: usize,
    /// Whether `IN` is not an operator here, as in the first argument of
    /// `POSITION(a IN b)`; see [`Parser::with_no_in`].
    no_in: bool,
}

impl<'t> Parser<'t> {
    pub(crate) fn new(source: &'t str, tokens: &'t [Token], significant: &'t [u32]) -> Self {
        Parser {
            source,
            tokens,
            significant,
            pos: 0,
            end: 0,
            events: Vec::with_capacity(significant.len() * 2 + 16),
            fallbacks: Vec::new(),
            preceded: Vec::new(),
            expected: None,
            depth: 0,
            budget: 0,
            no_in: false,
        }
    }

    /// Starts a statement: the parser reads the significant tokens
    /// `range` (indices into the text's significant tokens).
    pub(crate) fn set_range(&mut self, range: std::ops::Range<usize>) {
        self.pos = range.start;
        self.end = range.end;
        self.expected = None;
        self.depth = 0;
        self.no_in = false;
        self.budget = (range.end - range.start) * 4 + 4096;
    }

    pub(crate) fn finish(self) -> (Vec<Event>, Vec<Fallback>) {
        (self.events, self.fallbacks)
    }

    // ---- Looking ahead ----------------------------------------------------

    /// The `n`th significant token from the current one, within the
    /// statement.
    #[must_use]
    pub(crate) fn nth_token(&self, n: usize) -> Option<Token> {
        let i = self.pos + n;
        (i < self.end).then(|| self.tokens[self.significant[i] as usize])
    }

    /// The kind of the `n`th significant token from the current one.
    #[must_use]
    pub(crate) fn nth(&self, n: usize) -> Option<TokenKind> {
        self.nth_token(n).map(Token::kind)
    }

    /// Whether the current token is of `kind`.
    #[must_use]
    pub(crate) fn at(&self, kind: TokenKind) -> bool {
        self.nth(0) == Some(kind)
    }

    /// Whether the `n`th token is of `kind`.
    #[must_use]
    pub(crate) fn nth_at(&self, n: usize, kind: TokenKind) -> bool {
        self.nth(n) == Some(kind)
    }

    /// The keyword the `n`th token spells, if it is a bare word that is one.
    #[must_use]
    pub(crate) fn nth_keyword(&self, n: usize) -> Option<Keyword> {
        self.nth_token(n).and_then(Token::keyword)
    }

    /// Whether the current token is the bare word `keyword`, in any case.
    #[must_use]
    pub(crate) fn at_kw(&self, keyword: Keyword) -> bool {
        self.nth_keyword(0) == Some(keyword)
    }

    /// Whether the `n`th token is the bare word `keyword`.
    #[must_use]
    pub(crate) fn nth_at_kw(&self, n: usize, keyword: Keyword) -> bool {
        self.nth_keyword(n) == Some(keyword)
    }

    /// Whether the tokens from the current one spell `keywords`.
    #[must_use]
    pub(crate) fn at_kws(&self, keywords: &[Keyword]) -> bool {
        keywords
            .iter()
            .enumerate()
            .all(|(n, &k)| self.nth_at_kw(n, k))
    }

    /// The text of the `n`th token, or `""` past the end.
    #[must_use]
    pub(crate) fn nth_text(&self, n: usize) -> &'t str {
        self.nth_token(n).map_or("", |t| t.text(self.source))
    }

    /// Whether the statement has no more tokens.
    #[must_use]
    pub(crate) fn at_end(&self) -> bool {
        self.pos >= self.end
    }

    /// Whether the current token closes a bracket: `)`, `]` or `}`.
    #[must_use]
    pub(crate) fn at_closer(&self) -> bool {
        matches!(
            self.nth(0),
            Some(
                TokenKind::ClosingRoundBracket
                    | TokenKind::ClosingSquareBracket
                    | TokenKind::ClosingCurlyBrace
            )
        )
    }

    /// The position, for comparing how far two attempts got.
    #[must_use]
    pub(crate) fn position(&self) -> usize {
        self.pos
    }

    // ---- Consuming --------------------------------------------------------

    /// Consumes the current token as `kind`. Does nothing at the end.
    pub(crate) fn bump_as(&mut self, kind: SyntaxKind) {
        if self.pos < self.end {
            self.events.push(Event::Token {
                kind,
                index: self.significant[self.pos],
            });
            self.pos += 1;
        }
    }

    /// Consumes the current token as the lexer classified it: a bare word
    /// stays a [`BareWord`](SyntaxKind::BareWord), which is how names are
    /// consumed.
    pub(crate) fn bump(&mut self) {
        if let Some(kind) = self.nth(0) {
            self.bump_as(kind.into());
        }
    }

    /// Consumes the current token in a keyword slot: a bare word that is one
    /// of ClickHouse's keywords becomes a [`Keyword`](SyntaxKind::Keyword),
    /// or a [`LiteralWord`](SyntaxKind::LiteralWord) for `TRUE` and `FALSE`,
    /// whose canonical case is lower case wherever they stand (`IS TRUE`).
    /// Any other token is consumed as it is.
    pub(crate) fn bump_keyword(&mut self) {
        match self.nth_keyword(0) {
            Some(Keyword::True | Keyword::False) => self.bump_as(SyntaxKind::LiteralWord),
            Some(_) => self.bump_as(SyntaxKind::Keyword),
            None => self.bump(),
        }
    }

    /// Consumes the current token if it is of `kind`.
    pub(crate) fn eat(&mut self, kind: TokenKind) -> bool {
        let at = self.at(kind);
        if at {
            self.bump();
        }
        at
    }

    /// Consumes the current token as a keyword if it is `keyword`.
    pub(crate) fn eat_kw(&mut self, keyword: Keyword) -> bool {
        let at = self.at_kw(keyword);
        if at {
            self.bump_keyword();
        }
        at
    }

    /// Consumes `keywords` if the tokens from the current one spell them.
    #[allow(dead_code, reason = "for the statement grammar that builds on it")]
    pub(crate) fn eat_kws(&mut self, keywords: &[Keyword]) -> bool {
        let at = self.at_kws(keywords);
        if at {
            for _ in keywords {
                self.bump_keyword();
            }
        }
        at
    }

    /// Consumes a token of `kind`, or records that it was expected.
    pub(crate) fn expect(&mut self, kind: TokenKind) -> Option<()> {
        if self.eat(kind) {
            Some(())
        } else {
            self.expected(token_description(kind))
        }
    }

    /// Consumes the keyword `keyword`, or records that it was expected.
    pub(crate) fn expect_kw(&mut self, keyword: Keyword) -> Option<()> {
        if self.eat_kw(keyword) {
            Some(())
        } else {
            self.expected(keyword.as_str())
        }
    }

    /// Records that `what` was expected at the current token, and fails.
    ///
    /// The record is kept if no failure so far got further; a fallback uses
    /// it to say why it gave up.
    pub(crate) fn expected<T>(&mut self, what: &'static str) -> Option<T> {
        if self.expected.is_none_or(|e| self.pos >= e.pos) {
            self.expected = Some(Expected {
                pos: self.pos,
                what,
            });
        }
        None
    }

    // ---- Nodes ------------------------------------------------------------

    /// Starts a node whose kind is given when it completes.
    pub(crate) fn start(&mut self) -> Marker {
        let pos = self.events.len();
        self.events.push(Event::Start {
            kind: None,
            forward_parent: 0,
        });
        Marker { pos }
    }

    /// The kinds of the child nodes and tokens directly inside `node`, for
    /// grammar decisions that depend on what was parsed, such as whether a
    /// tuple can be a lambda's parameters.
    pub(crate) fn children(&self, node: CompletedMarker) -> Vec<SyntaxKind> {
        let mut out = Vec::new();
        let mut depth = 0usize;
        for event in self.events.get(node.start + 1..node.finish).unwrap_or(&[]) {
            match *event {
                Event::Start { kind, .. } => {
                    if depth == 0
                        && let Some(kind) = kind
                    {
                        out.push(kind);
                    }
                    depth += 1;
                }
                Event::Finish => depth = depth.saturating_sub(1),
                Event::Token { kind, .. } => {
                    if depth == 0 {
                        out.push(kind);
                    }
                }
            }
        }
        out
    }

    /// Turns the keyword slots inside `node` back into the lexer's bare
    /// words, for text that must be kept as written.
    pub(crate) fn demote_keywords(&mut self, node: CompletedMarker) {
        let range = node.start + 1..node.finish.min(self.events.len());
        for event in self.events.get_mut(range).unwrap_or(&mut []) {
            if let Event::Token { kind, .. } = event
                && matches!(kind, SyntaxKind::Keyword | SyntaxKind::LiteralWord)
            {
                *kind = SyntaxKind::BareWord;
            }
        }
    }

    /// The node kinds and token kinds anywhere inside `node`.
    pub(crate) fn descendants(&self, node: CompletedMarker) -> impl Iterator<Item = SyntaxKind> {
        self.events
            .get(node.start + 1..node.finish)
            .unwrap_or(&[])
            .iter()
            .filter_map(|event| match *event {
                Event::Start { kind, .. } => kind,
                Event::Token { kind, .. } => Some(kind),
                Event::Finish => None,
            })
    }

    // ---- Backtracking -----------------------------------------------------

    pub(crate) fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            events: self.events.len(),
            pos: self.pos,
            fallbacks: self.fallbacks.len(),
            preceded: self.preceded.len(),
            no_in: self.no_in,
        }
    }

    /// Returns to `checkpoint`: events, fallbacks and keyword slots recorded
    /// since are dropped, the parser's modes are as they were, and the tokens
    /// read since are charged to the speculation budget.
    pub(crate) fn rollback(&mut self, checkpoint: Checkpoint) {
        for &start in &self.preceded[checkpoint.preceded.min(self.preceded.len())..] {
            if let Some(Event::Start { forward_parent, .. }) = self.events.get_mut(start as usize) {
                *forward_parent = 0;
            }
        }
        self.preceded.truncate(checkpoint.preceded);
        self.events.truncate(checkpoint.events);
        self.fallbacks.truncate(checkpoint.fallbacks);
        self.no_in = checkpoint.no_in;
        self.budget = self
            .budget
            .saturating_sub(self.pos.saturating_sub(checkpoint.pos) + 1);
        self.pos = checkpoint.pos;
    }

    /// Tries `parse`; if it fails, returns to where it started.
    ///
    /// Returns `None` without trying once the statement's speculation budget,
    /// a few times its length, has gone on rollbacks: alternatives that are
    /// tried and given up on can then not add up to more than linear work,
    /// whatever the input.
    pub(crate) fn speculate<T>(&mut self, parse: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        if self.budget == 0 {
            return None;
        }
        let checkpoint = self.checkpoint();
        let expected = self.expected;
        let result = parse(self);
        if result.is_none() {
            self.rollback(checkpoint);
            // An alternative given up on is not why a later parse fails.
            self.expected = expected;
        }
        result
    }

    /// Whether `IN` is not an operator here.
    pub(crate) fn no_in(&self) -> bool {
        self.no_in
    }

    /// Runs `parse` with `IN` not an operator, if `no_in` is set, or an
    /// operator again, as inside brackets; the mode is restored however
    /// `parse` ends.
    pub(crate) fn with_no_in<T>(
        &mut self,
        no_in: bool,
        parse: impl FnOnce(&mut Self) -> Option<T>,
    ) -> Option<T> {
        let outer = std::mem::replace(&mut self.no_in, no_in);
        let result = parse(self);
        self.no_in = outer;
        result
    }

    /// Runs `parse` one nesting level deeper, or fails if that is deeper
    /// than [`MAX_DEPTH`].
    pub(crate) fn nested<T>(&mut self, parse: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        self.deeper(1, parse)
    }

    /// Runs `parse` `levels` nesting levels deeper.
    pub(crate) fn deeper<T>(
        &mut self,
        levels: u32,
        parse: impl FnOnce(&mut Self) -> Option<T>,
    ) -> Option<T> {
        if self.depth + levels > MAX_DEPTH {
            return self.expected("less deeply nested syntax");
        }
        self.depth += levels;
        let result = parse(self);
        self.depth -= levels;
        result
    }

    // ---- Fallbacks --------------------------------------------------------

    /// Records a fallback over the byte range of the tokens consumed since
    /// `checkpoint`, with the furthest expectation recorded since as its
    /// reason, and forgets that expectation.
    pub(crate) fn record_fallback(&mut self, level: FallbackLevel, checkpoint: Checkpoint) {
        let range = self.byte_range(checkpoint.pos, self.pos);
        let expected = self.expected.take().filter(|e| e.pos >= checkpoint.pos);
        let message = match expected {
            Some(e) => format!("expected {}, found {}", e.what, self.describe(e.pos)),
            None => format!("unexpected {}", self.describe(checkpoint.pos)),
        };
        self.fallbacks.push(Fallback {
            level,
            range,
            message,
        });
    }

    /// Forgets the furthest expectation, for a failure that is expected and
    /// handled, so that it does not show up as a later fallback's reason.
    pub(crate) fn clear_expected(&mut self) {
        self.expected = None;
    }

    fn byte_range(&self, from: usize, to: usize) -> std::ops::Range<usize> {
        let token = |i: usize| self.tokens[self.significant[i] as usize];
        if from >= to || from >= self.significant.len() {
            let at = self
                .significant
                .get(from)
                .map_or(self.source.len(), |&i| self.tokens[i as usize].start());
            return at..at;
        }
        token(from).start()..token(to - 1).end()
    }

    /// The significant token at `pos`, described for a message.
    fn describe(&self, pos: usize) -> String {
        if pos >= self.end {
            return "the end of the statement".to_owned();
        }
        let token = self.tokens[self.significant[pos] as usize];
        match (token.kind(), token.keyword()) {
            (_, Some(keyword)) => format!("keyword {keyword}"),
            (TokenKind::BareWord, None) => "a word".to_owned(),
            (TokenKind::Number, _) => "a number".to_owned(),
            (TokenKind::StringLiteral, _) => "a string".to_owned(),
            (TokenKind::QuotedIdentifier, _) => "a quoted identifier".to_owned(),
            (TokenKind::JinjaExpr, _) => "a Jinja expression".to_owned(),
            (kind, _) => token_description(kind).to_owned(),
        }
    }
}

/// How a message names a token kind: its text for punctuation.
pub(crate) fn token_description(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::OpeningRoundBracket => "`(`",
        TokenKind::ClosingRoundBracket => "`)`",
        TokenKind::OpeningSquareBracket => "`[`",
        TokenKind::ClosingSquareBracket => "`]`",
        TokenKind::OpeningCurlyBrace => "`{`",
        TokenKind::ClosingCurlyBrace => "`}`",
        TokenKind::Comma => "`,`",
        TokenKind::Semicolon => "`;`",
        TokenKind::Dot => "`.`",
        TokenKind::Asterisk => "`*`",
        TokenKind::Plus => "`+`",
        TokenKind::Minus => "`-`",
        TokenKind::Slash => "`/`",
        TokenKind::Percent => "`%`",
        TokenKind::Arrow => "`->`",
        TokenKind::QuestionMark => "`?`",
        TokenKind::Colon => "`:`",
        TokenKind::Caret => "`^`",
        TokenKind::DoubleColon => "`::`",
        TokenKind::Equals => "`=`",
        TokenKind::NotEquals => "`!=`",
        TokenKind::Less => "`<`",
        TokenKind::Greater => "`>`",
        TokenKind::LessOrEquals => "`<=`",
        TokenKind::GreaterOrEquals => "`>=`",
        TokenKind::Spaceship => "`<=>`",
        TokenKind::PipeMark => "`|`",
        TokenKind::PipeOperator => "`|>`",
        TokenKind::Concatenation => "`||`",
        TokenKind::Tilde => "`~`",
        TokenKind::TildeAsterisk => "`~*`",
        TokenKind::NotTilde => "`!~`",
        TokenKind::NotTildeAsterisk => "`!~*`",
        TokenKind::At => "`@`",
        TokenKind::DoubleAt => "`@@`",
        TokenKind::DollarSign => "`$`",
        TokenKind::VerticalDelimiter => "`\\G`",
        TokenKind::BareWord => "a word",
        TokenKind::Number => "a number",
        TokenKind::StringLiteral => "a string",
        TokenKind::QuotedIdentifier => "a quoted identifier",
        TokenKind::HereDoc => "a heredoc",
        TokenKind::JinjaExpr => "a Jinja expression",
        TokenKind::Verbatim => "a Jinja raw block",
        TokenKind::InsertData => "INSERT data",
        TokenKind::Whitespace
        | TokenKind::LineComment
        | TokenKind::BlockComment
        | TokenKind::JinjaBlock
        | TokenKind::JinjaComment => "trivia",
        _ => "a lexer error",
    }
}

/// An index or distance in the event list, which has a few entries per
/// token, and tokens have 32-bit offsets.
#[expect(
    clippy::cast_possible_truncation,
    reason = "event lists of inputs that fit in memory are far shorter than 4 Gi entries"
)]
fn event_offset(n: usize) -> u32 {
    n as u32
}

/// A node being parsed. Complete it with [`Marker::complete`] or give it up
/// with [`Marker::abandon`].
#[must_use]
#[derive(Debug)]
pub(crate) struct Marker {
    pos: usize,
}

impl Marker {
    /// Ends the node as `kind`.
    pub(crate) fn complete(self, p: &mut Parser<'_>, kind: SyntaxKind) -> CompletedMarker {
        match p.events.get_mut(self.pos) {
            Some(Event::Start { kind: slot, .. }) => *slot = Some(kind),
            _ => debug_assert!(false, "a marker outlived a rollback"),
        }
        p.events.push(Event::Finish);
        CompletedMarker {
            start: self.pos,
            finish: p.events.len() - 1,
            kind,
        }
    }

    /// Gives the node up; what was parsed inside it belongs to its parent.
    pub(crate) fn abandon(self, p: &mut Parser<'_>) {
        if self.pos + 1 == p.events.len() {
            p.events.pop();
        }
    }
}

/// A node that has been parsed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CompletedMarker {
    start: usize,
    finish: usize,
    kind: SyntaxKind,
}

impl CompletedMarker {
    /// Starts a node around this one, for syntax whose start is known only
    /// after its first part: `a` becomes the left operand of `a + b` when
    /// `+` is seen.
    pub(crate) fn precede(self, p: &mut Parser<'_>) -> Marker {
        let m = p.start();
        if let Some(Event::Start { forward_parent, .. }) = p.events.get_mut(self.start) {
            *forward_parent = event_offset(m.pos - self.start);
            p.preceded.push(event_offset(self.start));
        }
        m
    }

    /// The node's kind.
    pub(crate) fn kind(self) -> SyntaxKind {
        self.kind
    }
}
