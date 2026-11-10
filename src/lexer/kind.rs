//! Token kinds.

/// What a token is.
///
/// The SQL kinds mirror ClickHouse's `TokenType` (`src/Parsers/Lexer.h`), with
/// the same names, except that ClickHouse's `Comment` is split into
/// [`LineComment`](Self::LineComment) and [`BlockComment`](Self::BlockComment).
/// [`clickhouse_name`](Self::clickhouse_name) maps a kind back to ClickHouse's
/// name.
///
/// ClickHouse's `EndOfStream` has no counterpart (the token stream simply
/// ends), nor do `ErrorSinglePipeMark` and `ErrorMaxQuerySizeExceeded`, which
/// ClickHouse's lexer never produces for input without a size limit.
///
/// The `Jinja*` kinds, [`Verbatim`](Self::Verbatim) and the `ErrorJinja*`
/// kinds only occur in [Jinja mode](super::Mode::Jinja).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TokenKind {
    /// A run of whitespace: ASCII ` \t\n\r\f\v`, the Unicode spaces ClickHouse
    /// accepts (U+0085, U+00A0, U+180E, U+2000–U+200D, U+2028, U+2029, U+202F,
    /// U+205F, U+2060, U+3000) and the byte-order mark U+FEFF, in any mix.
    /// Adjacent whitespace is always one token.
    Whitespace,
    /// `-- …`, `// …`, `# …` or `#!…`, up to but not including the `\n` that
    /// ends its line, as ClickHouse lexes it:
    ///
    /// - the `\n` is never part of the comment, but a `\r` before it is, so
    ///   on CRLF input the comment ends in `\r`; so is any other whitespace at
    ///   the end of the line;
    /// - in [Jinja mode](super::Mode::Jinja) a tag inside the comment is
    ///   skipped whole, so a comment can span lines when a tag in it does
    ///   (`-- {{ x\n }}` is one comment); outside tags it never contains a
    ///   `\n`.
    LineComment,
    /// `/* … */`, nesting: `/* /* */ */` is one comment. It may span lines and
    /// keeps whatever line endings it has.
    BlockComment,

    /// A keyword or identifier: `[A-Za-z0-9_$]+`, which may start with a digit
    /// (`1abc`). Whether it is a keyword is the parser's call;
    /// [`Token::keyword`](super::Token::keyword) says whether its text is
    /// one of ClickHouse's keyword words.
    ///
    /// In Jinja mode, Jinja tags glued to a bare word or number join it into
    /// one `BareWord`, such as `events_{{ suffix }}` or
    /// `events{% if v2 %}_v2{% endif %}`, because the rendered text is one
    /// word and must not be split. Such a word is never a keyword.
    BareWord,
    /// A numeric literal, never signed: `1`, `1.5e-3`, `0x1F`, `0b101`, `.5`,
    /// `1_000`, `0x1.8p3`.
    Number,
    /// `'…'` (escapes: `''` and backslash), `‘…’` (no escapes), and hex or binary
    /// strings `x'…'`, `b'…'`.
    StringLiteral,
    /// `` `…` `` or `"…"` (escapes as for strings), or `“…”` (no escapes).
    QuotedIdentifier,

    /// `(`
    OpeningRoundBracket,
    /// `)`
    ClosingRoundBracket,
    /// `[`
    OpeningSquareBracket,
    /// `]`
    ClosingSquareBracket,
    /// `{`
    OpeningCurlyBrace,
    /// `}`
    ClosingCurlyBrace,
    /// `,`
    Comma,
    /// `;`
    Semicolon,
    /// `\G`
    VerticalDelimiter,
    /// `.` used as a qualifier or tuple access, as opposed to the start of a
    /// number like `.5`.
    Dot,
    /// `*`
    Asterisk,
    /// `$tag$…$tag$`, with a tag of `[A-Za-z0-9_]*`; it ends at the first exact
    /// closer.
    HereDoc,
    /// `$` not followed by a word character and not opening a heredoc.
    DollarSign,
    /// `+`
    Plus,
    /// `-`, or U+2212 MINUS SIGN.
    Minus,
    /// `/`
    Slash,
    /// `%`
    Percent,
    /// `->`
    Arrow,
    /// `?`
    QuestionMark,
    /// `:`
    Colon,
    /// `^`
    Caret,
    /// `::`
    DoubleColon,
    /// `=` or `==`
    Equals,
    /// `!=` or `<>`
    NotEquals,
    /// `<`
    Less,
    /// `>`
    Greater,
    /// `<=`
    LessOrEquals,
    /// `>=`
    GreaterOrEquals,
    /// `<=>`
    Spaceship,
    /// `|`
    PipeMark,
    /// `|>`
    PipeOperator,
    /// `||`
    Concatenation,
    /// `~`
    Tilde,
    /// `~*`
    TildeAsterisk,
    /// `!~`
    NotTilde,
    /// `!~*`
    NotTildeAsterisk,
    /// `@`
    At,
    /// `@@`
    DoubleAt,

    /// `{{ … }}`, an opaque atom that stands for an expression or a name.
    ///
    /// Several glued tags with no SQL word between them that include an
    /// expression, such as `{{ schema }}{{ suffix }}` or
    /// `{% if a %}{{ b }}{% endif %}`, are one `JinjaExpr`, since the rendered
    /// text of the expressions is glued too.
    JinjaExpr,
    /// `{% … %}`, a statement tag. It renders no text of its own, so the parser
    /// steps over it like a comment.
    JinjaBlock,
    /// `{# … #}`, a Jinja comment.
    JinjaComment,
    /// `{% raw %}…{% endraw %}`: text the template renders as written, which
    /// is never looked inside and stands as an opaque atom.
    Verbatim,

    /// An unrecognised character, with its UTF-8 continuation bytes: `#` not
    /// followed by a space or `!`, `\` not followed by `G`, or a non-ASCII
    /// character that is not whitespace.
    Error,
    /// `/*` without its closer; runs to the end of the input.
    ErrorMultilineCommentIsNotClosed,
    /// `'`, `‘` or `x'` without its closer (or a hex or binary string with a
    /// character that does not belong in it); runs to the end of the input.
    ErrorSingleQuoteIsNotClosed,
    /// `"` or `“` without its closer; runs to the end of the input.
    ErrorDoubleQuoteIsNotClosed,
    /// `` ` `` without its closer; runs to the end of the input.
    ErrorBackQuoteIsNotClosed,
    /// `!` not followed by `=` or `~`.
    ErrorSingleExclamationMark,
    /// A number followed by word characters that do not make a bare word, such
    /// as `1e+x` or `1.5abc`.
    ErrorWrongNumber,
    /// `{{`, `{%` or `{#` without its closer; runs to the end of the input.
    /// A line comment with such a tag in it is this error too, from the
    /// comment marker on: the renderer reads the tag regardless of the
    /// comment, so the comment never ends at its line.
    ErrorJinjaTagIsNotClosed,
    /// `{% raw %}` without `{% endraw %}`; runs to the end of the input.
    ErrorJinjaRawIsNotClosed,
}

impl TokenKind {
    /// Tokens the parser steps over: whitespace, comments, and the Jinja tags
    /// that render no text ([`JinjaBlock`](Self::JinjaBlock) and
    /// [`JinjaComment`](Self::JinjaComment)).
    ///
    /// For SQL this is ClickHouse's notion of an insignificant token. Block
    /// tags count as trivia so that the parser sees the SQL of every branch
    /// of an `{% if %}` at once.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Whitespace
                | Self::LineComment
                | Self::BlockComment
                | Self::JinjaBlock
                | Self::JinjaComment
        )
    }

    /// Any token that is not [trivia](Self::is_trivia).
    #[must_use]
    pub const fn is_significant(self) -> bool {
        !self.is_trivia()
    }

    /// A SQL line or block comment.
    #[must_use]
    pub const fn is_comment(self) -> bool {
        matches!(self, Self::LineComment | Self::BlockComment)
    }

    /// A token that consists of Jinja tags: [`JinjaExpr`](Self::JinjaExpr),
    /// [`JinjaBlock`](Self::JinjaBlock), [`JinjaComment`](Self::JinjaComment)
    /// or [`Verbatim`](Self::Verbatim). Tags also occur glued into a
    /// [`BareWord`](Self::BareWord) and inside strings, quoted identifiers,
    /// heredocs and comments; [`jinja::tags`](super::jinja::tags) finds them
    /// in any token.
    #[must_use]
    pub const fn is_jinja(self) -> bool {
        matches!(
            self,
            Self::JinjaExpr | Self::JinjaBlock | Self::JinjaComment | Self::Verbatim
        )
    }

    /// An error token. After one, the identity of the following tokens cannot
    /// be guaranteed, so a formatter must leave the input unchanged.
    #[must_use]
    pub const fn is_error(self) -> bool {
        matches!(
            self,
            Self::Error
                | Self::ErrorMultilineCommentIsNotClosed
                | Self::ErrorSingleQuoteIsNotClosed
                | Self::ErrorDoubleQuoteIsNotClosed
                | Self::ErrorBackQuoteIsNotClosed
                | Self::ErrorSingleExclamationMark
                | Self::ErrorWrongNumber
                | Self::ErrorJinjaTagIsNotClosed
                | Self::ErrorJinjaRawIsNotClosed
        )
    }

    /// The name of ClickHouse's `TokenType` for this kind, or `None` for a kind
    /// ClickHouse does not have.
    ///
    /// ```
    /// use chairlift::lexer::TokenKind;
    ///
    /// assert_eq!(TokenKind::LineComment.clickhouse_name(), Some("Comment"));
    /// assert_eq!(TokenKind::BareWord.clickhouse_name(), Some("BareWord"));
    /// ```
    #[must_use]
    pub const fn clickhouse_name(self) -> Option<&'static str> {
        Some(match self {
            Self::Whitespace => "Whitespace",
            Self::LineComment | Self::BlockComment => "Comment",
            Self::BareWord => "BareWord",
            Self::Number => "Number",
            Self::StringLiteral => "StringLiteral",
            Self::QuotedIdentifier => "QuotedIdentifier",
            Self::OpeningRoundBracket => "OpeningRoundBracket",
            Self::ClosingRoundBracket => "ClosingRoundBracket",
            Self::OpeningSquareBracket => "OpeningSquareBracket",
            Self::ClosingSquareBracket => "ClosingSquareBracket",
            Self::OpeningCurlyBrace => "OpeningCurlyBrace",
            Self::ClosingCurlyBrace => "ClosingCurlyBrace",
            Self::Comma => "Comma",
            Self::Semicolon => "Semicolon",
            Self::VerticalDelimiter => "VerticalDelimiter",
            Self::Dot => "Dot",
            Self::Asterisk => "Asterisk",
            Self::HereDoc => "HereDoc",
            Self::DollarSign => "DollarSign",
            Self::Plus => "Plus",
            Self::Minus => "Minus",
            Self::Slash => "Slash",
            Self::Percent => "Percent",
            Self::Arrow => "Arrow",
            Self::QuestionMark => "QuestionMark",
            Self::Colon => "Colon",
            Self::Caret => "Caret",
            Self::DoubleColon => "DoubleColon",
            Self::Equals => "Equals",
            Self::NotEquals => "NotEquals",
            Self::Less => "Less",
            Self::Greater => "Greater",
            Self::LessOrEquals => "LessOrEquals",
            Self::GreaterOrEquals => "GreaterOrEquals",
            Self::Spaceship => "Spaceship",
            Self::PipeMark => "PipeMark",
            Self::PipeOperator => "PipeOperator",
            Self::Concatenation => "Concatenation",
            Self::Tilde => "Tilde",
            Self::TildeAsterisk => "TildeAsterisk",
            Self::NotTilde => "NotTilde",
            Self::NotTildeAsterisk => "NotTildeAsterisk",
            Self::At => "At",
            Self::DoubleAt => "DoubleAt",
            Self::Error => "Error",
            Self::ErrorMultilineCommentIsNotClosed => "ErrorMultilineCommentIsNotClosed",
            Self::ErrorSingleQuoteIsNotClosed => "ErrorSingleQuoteIsNotClosed",
            Self::ErrorDoubleQuoteIsNotClosed => "ErrorDoubleQuoteIsNotClosed",
            Self::ErrorBackQuoteIsNotClosed => "ErrorBackQuoteIsNotClosed",
            Self::ErrorSingleExclamationMark => "ErrorSingleExclamationMark",
            Self::ErrorWrongNumber => "ErrorWrongNumber",
            Self::JinjaExpr
            | Self::JinjaBlock
            | Self::JinjaComment
            | Self::Verbatim
            | Self::ErrorJinjaTagIsNotClosed
            | Self::ErrorJinjaRawIsNotClosed => return None,
        })
    }

    /// A description of an error kind, worded as ClickHouse's
    /// `getErrorTokenDescription` for its kinds, or `None` for a kind that is
    /// not an error.
    #[must_use]
    pub const fn error_description(self) -> Option<&'static str> {
        Some(match self {
            Self::Error => "Unrecognized token",
            Self::ErrorMultilineCommentIsNotClosed => "Multiline comment is not closed",
            Self::ErrorSingleQuoteIsNotClosed => "Single quoted string is not closed",
            Self::ErrorDoubleQuoteIsNotClosed => "Double quoted string is not closed",
            Self::ErrorBackQuoteIsNotClosed => "Back quoted string is not closed",
            Self::ErrorSingleExclamationMark => {
                "Exclamation mark can only occur in !=, !~ and !~* operators"
            }
            Self::ErrorWrongNumber => "Wrong number",
            Self::ErrorJinjaTagIsNotClosed => "Jinja tag is not closed",
            Self::ErrorJinjaRawIsNotClosed => "Jinja raw block is not closed",
            _ => return None,
        })
    }
}
