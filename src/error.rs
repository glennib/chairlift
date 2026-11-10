//! Why a text could not be formatted.

use crate::Position;
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
    /// The formatted text failed [verification](crate::verify): a bug in
    /// chairlift, which kept the text from being changed.
    #[error("the formatted text failed verification, a bug in chairlift: {0}")]
    Verify(#[from] VerifyError),
    /// The text is 4 GiB or longer.
    #[error("the text is {len} bytes long; chairlift formats texts under 4 GiB")]
    TooLarge {
        /// The text's length in bytes.
        len: usize,
    },
}

/// A lexer error: the first [error token](TokenKind::is_error) of a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{position}: {}", kind.error_description().unwrap_or("lexer error"))]
pub struct LexError {
    /// Where the error token starts.
    pub position: Position,
    /// The error token's kind.
    pub kind: TokenKind,
}
