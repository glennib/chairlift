//! Why a text could not be formatted.

use crate::Position;
use crate::doc::TokenCheckError;
use crate::lexer::TokenKind;
use crate::verify::VerifyError;

/// Why [`format`](crate::format) left a text unformatted.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The text has a lexer error, so what its tokens are after that point
    /// cannot be relied on.
    #[error(transparent)]
    Lex(#[from] LexError),
    /// The formatted text failed verification: a bug in chairlift, which
    /// kept the text from being changed.
    #[error("the formatted text failed verification, a bug in chairlift: {0}")]
    Verify(#[from] VerifyError),
    /// The layout did not print every token of the text exactly once: a
    /// bug in chairlift, which kept the text from being changed.
    #[error("the layout lost or repeated a token, a bug in chairlift: {0}")]
    Layout(#[from] TokenCheckError),
    /// The text is a template in which a line comment can run on past the
    /// line break that ends it, in a render whose result formatting could
    /// change: what the comment swallows there is decided by where the
    /// layout puts what follows it, so the text is left as it is.
    #[error(transparent)]
    Template(#[from] TemplateError),
    /// The text is 4 GiB or longer.
    #[error("the text is {len} bytes long; chairlift formats texts under 4 GiB")]
    TooLarge {
        /// The text's length in bytes.
        len: usize,
    },
}

/// A line comment of a template whose end the rendering can move (see
/// [`Error::Template`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{position}: where this line comment ends depends on the rendering: {reason}")]
pub struct TemplateError {
    /// Where the comment starts.
    pub position: Position,
    /// What can remove the line break that ends it.
    pub reason: CommentEnd,
}

/// What removes the line break that ends a template's line comment, in a
/// render whose result formatting could change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum CommentEnd {
    /// A block or comment tag inside the comment, whose block can end past
    /// it: `-- {% if x %}` and the line after.
    #[error(
        "a block or comment tag inside it can remove the line break after it; \
         move the tag out of the comment"
    )]
    TagInside,
    /// A `-` marker on a tag at the end of the comment: `-- c {{ v -}}`.
    #[error(
        "the `-` marker of the tag at its end strips the line break after it; remove the marker"
    )]
    MarkerInside,
    /// A `-` marker on the tag after the comment, when the render can then
    /// go on with text on the line of a tag rather than after its line break:
    /// `-- c` and `{%- if x %} b`.
    #[error(
        "the `-` marker of the tag after it strips the line break after it, and the comment \
         can run on into text that follows a tag on its line; remove the marker, or end the \
         lines of the tags there"
    )]
    MarkerAfter,
    /// The comment ends a macro's body, and the `-` marker of
    /// `{%- endmacro %}` strips the line break after it: the comment runs
    /// on into whatever follows each call of the macro.
    #[error(
        "it ends a macro's body and the `-` marker of the tag after it strips the line \
         break, so it runs on into what follows each call; remove the marker"
    )]
    MacroBody,
}

/// A lexer error: the first error token of a text, such as an unclosed
/// string or comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{position}: {}", self.description())]
pub struct LexError {
    /// Where the error token starts.
    pub position: Position,
    pub(crate) kind: TokenKind,
}

impl LexError {
    /// The error at `position`, of an error token of `kind`.
    #[cfg(feature = "unstable")]
    #[must_use]
    pub fn new(position: Position, kind: TokenKind) -> LexError {
        LexError { position, kind }
    }

    /// What is wrong, as ClickHouse's lexer words it: `Single quoted string
    /// is not closed`.
    #[must_use]
    pub fn description(&self) -> &'static str {
        self.kind.error_description().unwrap_or("lexer error")
    }

    /// The error token's kind.
    #[cfg(feature = "unstable")]
    #[must_use]
    pub fn kind(&self) -> TokenKind {
        self.kind
    }
}
