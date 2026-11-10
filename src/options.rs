//! Formatting options.

use crate::lexer::Mode;

/// How to format.
///
/// The struct is `non_exhaustive` so that options can be added: start from
/// [`Options::default`] and set what differs.
///
/// ```
/// let options = chairlift::Options::default().with_jinja(true);
/// assert_eq!(options.line_width, 100);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Options {
    /// The width lines are laid out to fit, in `char`s.
    pub line_width: u16,
    /// Whether the text is a Jinja template (see [`Mode::Jinja`]).
    pub jinja: bool,
}

impl Options {
    /// The default [`line_width`](Self::line_width).
    pub const DEFAULT_LINE_WIDTH: u16 = 100;

    /// These options with `line_width` set.
    #[must_use]
    pub fn with_line_width(mut self, line_width: u16) -> Self {
        self.line_width = line_width;
        self
    }

    /// These options with `jinja` set.
    #[must_use]
    pub fn with_jinja(mut self, jinja: bool) -> Self {
        self.jinja = jinja;
        self
    }

    /// The lexer mode the options ask for.
    #[must_use]
    pub fn mode(&self) -> Mode {
        if self.jinja { Mode::Jinja } else { Mode::Sql }
    }
}

impl Default for Options {
    fn default() -> Self {
        Options {
            line_width: Self::DEFAULT_LINE_WIDTH,
            jinja: false,
        }
    }
}
