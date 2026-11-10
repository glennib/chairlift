//! Splitting a source text into statements.
//!
//! [`split`] lexes a text and groups its tokens into [`Statement`]s the way
//! `clickhouse-client` does when it runs a file of queries, which differs from
//! `clickhouse-format` only in how it finds the end of an `INSERT`'s inline
//! data:
//!
//! - every `;` token ends a statement; a `;` inside a string, quoted
//!   identifier, heredoc, comment or Jinja tag is part of that token and never
//!   does;
//! - the data after `INSERT … VALUES` (or `FORMAT Values`) runs up to the
//!   `;` that ends the statement, or the end of the input;
//! - the data after `INSERT … FORMAT name` for any other format starts after
//!   the format name, past spaces, tabs and form feeds and then one line
//!   break, and runs up to the first blank line (`\n\n`) or the end of the
//!   input; in Jinja mode, the first blank line outside tags, since the
//!   renderer replaces the tags. Such a statement has no `;`: what follows the data is the next
//!   statement. `INSERT … SELECT … FORMAT name` has data only when its
//!   `SELECT` reads from the `input` table function, and `FROM INFILE` has
//!   none. A `format` that leads to no data, such as a column or function
//!   of that name in the `SELECT`, does not end the search: the `FORMAT`
//!   clause may still follow.
//! - in Jinja mode, an `INSERT` right after a `{{ … }}` expression starts an
//!   `INSERT` head, since the expression can render as whole statements.
//!
//! ClickHouse does not lex inline data, so neither does the splitter: each
//! data region is a single [`InsertData`](TokenKind::InsertData) token, and
//! lexing resumes after it as ClickHouse does, with a fresh lexer. Apart from
//! that, the tokens are the lexer's (see [`crate::lexer`]), and they keep its
//! guarantees: they cover the text exactly, none is empty, and adjacent
//! whitespace is one token.
//!
//! ```
//! use chairlift::lexer::{Mode, TokenKind};
//! use chairlift::split::{DataFormat, split};
//!
//! let source = "INSERT INTO t FORMAT CSV\n1,'a;b'\n\nSELECT 1; SELECT 2";
//! let split = split(source, Mode::Sql);
//! let [insert, select_1, select_2] = split.statements() else { panic!() };
//! let data = insert.data.unwrap();
//! assert_eq!(data.format, DataFormat::Other);
//! assert_eq!(split.tokens()[data.token].text(source), "1,'a;b'");
//! assert_eq!(split.text(select_1, source), "SELECT 1;");
//! assert_eq!(select_2.terminator, None);
//! ```

use std::ops::Range;

use crate::lexer::Lexer;
use crate::lexer::Mode;
use crate::lexer::Token;
use crate::lexer::TokenKind;
use crate::lexer::jinja;

/// A text's tokens, grouped into statements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Split {
    tokens: Vec<Token>,
    statements: Vec<Statement>,
    /// An unclosed Jinja tag inside data that is not lexed.
    data_error: Option<Token>,
}

/// A statement: a run of tokens from its first significant token through the
/// `;` that ends it.
///
/// Trivia (whitespace, comments, Jinja block and comment tags) before a
/// statement's first significant token or after its last is not part of it;
/// trivia between its tokens is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    /// Indices into [`Split::tokens`]: from the first significant token up to
    /// and including the terminating `;`, or the last significant token when
    /// there is no `;`.
    pub tokens: Range<usize>,
    /// The index of the `;` that ends the statement, if there is one. A
    /// statement that is only a `;` is empty.
    pub terminator: Option<usize>,
    /// The `INSERT`'s inline data, if the statement has any.
    pub data: Option<InsertData>,
}

/// The inline data of an `INSERT` statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InsertData {
    /// The index of its [`InsertData`](TokenKind::InsertData) token.
    pub token: usize,
    /// How ClickHouse finds where it ends.
    pub format: DataFormat,
}

/// Which of `clickhouse-client`'s two rules ends an `INSERT`'s data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataFormat {
    /// `VALUES`, or `FORMAT Values`: rows up to the terminating `;`. The data
    /// token spans the rows, from the first significant token after `VALUES`
    /// to the last token before the `;` that is not whitespace.
    Values,
    /// Any other format: text up to the first blank line.
    Other,
}

impl Split {
    /// The tokens, covering the text exactly.
    #[must_use]
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// The statements, in source order.
    #[must_use]
    pub fn statements(&self) -> &[Statement] {
        &self.statements
    }

    /// The text of `statement`, from its first token to its last; `source` is
    /// the text that was split.
    ///
    /// # Panics
    ///
    /// If `statement` or `source` is not from this split.
    #[must_use]
    pub fn text<'s>(&self, statement: &Statement, source: &'s str) -> &'s str {
        let first = self.tokens[statement.tokens.start];
        let last = self.tokens[statement.tokens.end - 1];
        &source[first.start()..last.end()]
    }

    /// The first [error token](TokenKind::is_error), if there is one.
    ///
    /// Data in a format other than `Values` is not lexed, so what would be
    /// error tokens there do not count. Jinja tags inside it still do, since
    /// the renderer reads them: in Jinja mode, an unclosed tag inside such data
    /// is reported as an
    /// [`ErrorJinjaTagIsNotClosed`](TokenKind::ErrorJinjaTagIsNotClosed) token
    /// over the tag, which is not one of [`tokens`](Self::tokens).
    #[must_use]
    pub fn first_error(&self) -> Option<Token> {
        let token = self.tokens.iter().copied().find(|t| t.kind().is_error());
        match (token, self.data_error) {
            (Some(a), Some(b)) => Some(if a.start() < b.start() { a } else { b }),
            (a, b) => a.or(b),
        }
    }
}

impl Statement {
    /// Whether the statement is only a `;`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.terminator == Some(self.tokens.start)
    }
}

/// Lexes `source` and splits it into statements; see the
/// [module documentation](self).
///
/// # Panics
///
/// If `source` is 4 GiB or longer, as [`Lexer::new`] does.
#[must_use]
pub fn split(source: &str, mode: Mode) -> Split {
    let mut splitter = Splitter {
        source,
        mode,
        lexer: Lexer::new(source, mode),
        base: 0,
        tokens: Vec::with_capacity(source.len() / 4 + 1),
        statements: Vec::new(),
        data_error: None,
    };
    while let Some(first) = splitter.next_significant() {
        let statement = splitter.statement(first);
        splitter.statements.push(statement);
    }
    Split {
        tokens: splitter.tokens,
        statements: splitter.statements,
        data_error: splitter.data_error,
    }
}

struct Splitter<'s> {
    source: &'s str,
    mode: Mode,
    /// Lexes `source[base..]`.
    lexer: Lexer<'s>,
    base: usize,
    tokens: Vec<Token>,
    statements: Vec<Statement>,
    data_error: Option<Token>,
}

impl Splitter<'_> {
    /// Lexes the next token and returns its index, merging it into the
    /// previous token when both are whitespace (which happens only where
    /// lexing restarted).
    fn pull(&mut self) -> Option<usize> {
        let token = self.lexer.next()?.shifted(self.base);
        self.push(token);
        Some(self.tokens.len() - 1)
    }

    fn push(&mut self, token: Token) {
        if token.kind() == TokenKind::Whitespace
            && let Some(last) = self.tokens.last_mut()
            && last.kind() == TokenKind::Whitespace
        {
            *last = last.extended_to(token.end());
        } else {
            self.tokens.push(token);
        }
    }

    fn next_significant(&mut self) -> Option<usize> {
        loop {
            let i = self.pull()?;
            if self.tokens[i].kind().is_significant() {
                return Some(i);
            }
        }
    }

    /// Lexes the rest of the text afresh from `at`, as ClickHouse does after
    /// inline data.
    fn restart(&mut self, at: usize) {
        self.lexer = Lexer::new(&self.source[at..], self.mode);
        self.base = at;
    }

    /// The statement whose first significant token is `first`.
    fn statement(&mut self, first: usize) -> Statement {
        let mut head = Head::default();
        let mut i = first;
        loop {
            let token = self.tokens[i];
            if token.kind() == TokenKind::Semicolon {
                return Statement {
                    tokens: first..i + 1,
                    terminator: Some(i),
                    data: None,
                };
            }
            match head.feed(token, token.text(self.source)) {
                Step::Continue => {}
                Step::Values => return self.values(first, i),
                Step::Format => {
                    if let Some(statement) = self.format_data(first, i) {
                        return statement;
                    }
                }
            }
            match self.next_significant() {
                Some(next) => i = next,
                None => {
                    return Statement {
                        tokens: first..i + 1,
                        terminator: None,
                        data: None,
                    };
                }
            }
        }
    }

    /// The rest of a statement whose `VALUES` (or `FORMAT Values`) is token
    /// `values`: rows up to the next `;`.
    fn values(&mut self, first: usize, values: usize) -> Statement {
        let Some(start) = self.next_significant() else {
            return Statement {
                tokens: first..values + 1,
                terminator: None,
                data: None,
            };
        };
        if self.tokens[start].kind() == TokenKind::Semicolon {
            return Statement {
                tokens: first..start + 1,
                terminator: Some(start),
                data: None,
            };
        }
        // The last token that is not whitespace, and the last significant one.
        let mut last = start;
        let mut last_significant = start;
        let mut terminator = None;
        let mut has_error = self.tokens[start].kind().is_error();
        while let Some(j) = self.pull() {
            let kind = self.tokens[j].kind();
            if kind == TokenKind::Semicolon {
                terminator = Some(j);
                break;
            }
            if kind != TokenKind::Whitespace {
                last = j;
            }
            if kind.is_significant() {
                last_significant = j;
            }
            has_error |= kind.is_error();
        }
        if has_error {
            // The rows' extent is unknown, so the text cannot be formatted;
            // the error tokens stay visible.
            return Statement {
                tokens: first..terminator.unwrap_or(last_significant) + 1,
                terminator,
                data: None,
            };
        }
        let range = self.tokens[start].start()..self.tokens[last].end();
        self.tokens.drain(start + 1..=last);
        self.tokens[start] = Token::synthetic(TokenKind::InsertData, range);
        let removed = last - start;
        Statement {
            tokens: first..terminator.map_or(start, |t| t - removed) + 1,
            terminator: terminator.map(|t| t - removed),
            data: Some(InsertData {
                token: start,
                format: DataFormat::Values,
            }),
        }
    }

    /// For a statement whose format name is token `name`, the statement with
    /// the data that follows it, or `None` if no data follows (the format
    /// name is then followed by `;` or the end of the input).
    fn format_data(&mut self, first: usize, name: usize) -> Option<Statement> {
        let bytes = self.source.as_bytes();
        let name_end = self.tokens[name].end();
        let mut start = name_end;
        while bytes
            .get(start)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\x0c'))
        {
            start += 1;
        }
        if bytes.get(start) == Some(&b'\r') {
            start += 1;
        }
        if bytes.get(start) == Some(&b'\n') {
            start += 1;
        }
        if start == bytes.len() || bytes[start] == b';' {
            return None;
        }
        let end = self.data_end(start);
        if start > name_end {
            self.push(Token::synthetic(TokenKind::Whitespace, name_end..start));
        }
        let data = if end > start {
            self.tokens
                .push(Token::synthetic(TokenKind::InsertData, start..end));
            Some(InsertData {
                token: self.tokens.len() - 1,
                format: DataFormat::Other,
            })
        } else {
            None
        };
        self.restart(end);
        Some(Statement {
            tokens: first..data.map_or(name + 1, |d| d.token + 1),
            terminator: None,
            data,
        })
    }
}

impl Splitter<'_> {
    /// Where data in a format other than `Values` that starts at `start`
    /// ends: at the first `\n\n`, or the end of the input.
    ///
    /// In Jinja mode the `\n\n` must be outside tags, since the renderer
    /// replaces them before ClickHouse sees the data, and an unclosed tag
    /// runs to the end of the input and is recorded as an error.
    fn data_end(&mut self, start: usize) -> usize {
        let rest = &self.source[start..];
        let pair = |from: usize, to: usize| {
            memchr_pair(&rest.as_bytes()[from..to]).map(|p| start + from + p)
        };
        if self.mode == Mode::Sql {
            return pair(0, rest.len()).unwrap_or(self.source.len());
        }
        let mut from = 0;
        for tag in jinja::tags(rest) {
            if let Some(end) = pair(from, tag.range.start) {
                return end;
            }
            if !tag.closed {
                self.data_error = Some(Token::synthetic(
                    TokenKind::ErrorJinjaTagIsNotClosed,
                    start + tag.range.start..self.source.len(),
                ));
                return self.source.len();
            }
            from = tag.range.end;
        }
        pair(from, rest.len()).unwrap_or(self.source.len())
    }
}

/// The offset of the first `\n\n` in `bytes`.
fn memchr_pair(bytes: &[u8]) -> Option<usize> {
    bytes.windows(2).position(|w| w == b"\n\n")
}

/// What the statement splitter does after a significant token.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Continue,
    /// The token is `VALUES`, or the format name `Values`: rows follow.
    Values,
    /// The token is the name of a format other than `Values`, and data
    /// follows it.
    Format,
}

/// Where the splitter is in an `INSERT` statement's head, following
/// `ParserInsertQuery` closely enough to find where its data starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    /// Before the first significant token.
    #[default]
    Start,
    /// After `WITH` or `EXPLAIN`, which may precede `INSERT INTO`.
    Prefix,
    /// After `INSERT`.
    Insert,
    /// After `INSERT INTO [TABLE]`.
    Into,
    /// After `FUNCTION`, up to the end of the table function's arguments.
    Function,
    /// After the table name.
    Table,
    /// After `database.`.
    Qualified,
    /// After the `(` of the column list.
    ColumnsStart,
    /// Inside the column list.
    Columns,
    /// After the target and the column list.
    Rest,
    /// After `FORMAT`.
    FormatName,
    /// Not an `INSERT`, or past the point where data could start.
    Done,
}

#[derive(Debug, Default)]
struct Head {
    state: State,
    /// Bracket depth.
    depth: u32,
    prev: Prev,
    infile: bool,
    select: bool,
    /// The `SELECT` reads from the `input` table function.
    input: bool,
}

/// What the previous significant token was, as far as the head cares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Prev {
    #[default]
    Other,
    /// `=`: this token is a setting's value.
    Equals,
    /// `FROM`: `INFILE` or a `SELECT … FROM` follows.
    From,
    /// The bare word `input`, which with a `(` is the table function.
    Input,
    /// Another name: a `(` after it is a call.
    Name,
    /// A Jinja expression, which is a name or anything else.
    Expr,
}

impl Head {
    fn feed(&mut self, token: Token, text: &str) -> Step {
        let kind = token.kind();
        let word = |w: &str| kind == TokenKind::BareWord && text.eq_ignore_ascii_case(w);
        let is_name = matches!(
            kind,
            TokenKind::BareWord | TokenKind::QuotedIdentifier | TokenKind::JinjaExpr
        );
        let opens = matches!(
            kind,
            TokenKind::OpeningRoundBracket | TokenKind::OpeningSquareBracket
        );
        let closes = matches!(
            kind,
            TokenKind::ClosingRoundBracket | TokenKind::ClosingSquareBracket
        );
        let depth = self.depth;
        if opens {
            self.depth += 1;
        } else if closes {
            self.depth = self.depth.saturating_sub(1);
        }
        let prev = std::mem::replace(
            &mut self.prev,
            if kind == TokenKind::Equals {
                Prev::Equals
            } else if word("FROM") {
                Prev::From
            } else if word("input") {
                Prev::Input
            } else if kind == TokenKind::JinjaExpr {
                Prev::Expr
            } else if is_name {
                Prev::Name
            } else {
                Prev::Other
            },
        );
        let call = kind == TokenKind::OpeningRoundBracket
            && matches!(prev, Prev::Name | Prev::Input | Prev::Expr);
        if call && prev == Prev::Input {
            self.input = true;
        }

        let mut step = Step::Continue;
        self.state = match self.state {
            // An expression can render as whole statements, `;` included, so
            // an `INSERT` after one may start a statement of the rendered SQL.
            _ if prev == Prev::Expr && depth == 0 && word("INSERT") => {
                self.infile = false;
                self.select = false;
                self.input = false;
                State::Insert
            }
            State::Start if kind == TokenKind::JinjaExpr => State::Start,
            State::Start if word("INSERT") => State::Insert,
            State::Start if word("WITH") || word("EXPLAIN") => State::Prefix,
            State::Prefix if depth == 0 && word("INSERT") => State::Insert,
            State::Prefix if depth == 0 && word("SELECT") => State::Done,
            State::Prefix => State::Prefix,
            State::Insert if word("INTO") => State::Into,
            State::Into if word("TABLE") => State::Into,
            State::Into if word("FUNCTION") => State::Function,
            State::Into | State::Qualified if is_name => State::Table,
            State::Function if closes && self.depth == 0 => State::Rest,
            State::Function => State::Function,
            State::Table if kind == TokenKind::Dot => State::Qualified,
            State::Table | State::Qualified if kind == TokenKind::OpeningRoundBracket => {
                State::ColumnsStart
            }
            State::ColumnsStart | State::Columns => {
                // `INSERT INTO t (SELECT …)` is a parenthesised SELECT, which
                // ClickHouse finds by failing to read a column list.
                if self.state == State::ColumnsStart && (word("SELECT") || word("WITH") || opens) {
                    self.select = true;
                }
                if self.depth == 0 {
                    State::Rest
                } else {
                    State::Columns
                }
            }
            State::Table | State::Qualified | State::Rest => {
                if prev == Prev::From {
                    if word("INFILE") {
                        self.infile = true;
                    } else {
                        self.select = true;
                    }
                }
                if depth > 0 || prev == Prev::Equals || call {
                    State::Rest
                } else if word("VALUES") && !self.infile && !self.select {
                    step = Step::Values;
                    State::Done
                } else if word("FORMAT") {
                    State::FormatName
                } else {
                    if word("SELECT") || word("WITH") || kind == TokenKind::OpeningRoundBracket {
                        self.select = true;
                    }
                    State::Rest
                }
            }
            State::FormatName if is_name => {
                if !self.infile && (!self.select || self.input) {
                    step = if text == "Values" {
                        Step::Values
                    } else {
                        Step::Format
                    };
                }
                // A column or function named `format` in the `SELECT` is no
                // `FORMAT` clause: the real one may still come.
                State::Rest
            }
            State::FormatName => State::Rest,
            _ => State::Done,
        };
        step
    }
}

#[cfg(test)]
mod tests;
