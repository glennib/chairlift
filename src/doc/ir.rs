//! The flat representation of a document: nodes in output order, with
//! groups and scopes as start and end markers, and the side tables they
//! point into.

use super::Gap;

/// "No index" in a `u32` field.
pub(super) const NONE: u32 = u32::MAX;

/// One node of a document.
#[derive(Clone, Copy, Debug)]
pub(super) enum Node {
    /// A token of the input.
    Atom(Atom),
    /// A run of input tokens printed as written: an index into
    /// [`Doc::verbatims`](super::Doc).
    Verbatim(u32),
    /// A layout space.
    Space,
    /// A line break opportunity, or a line break.
    Line(LineKind),
    /// Keep up to this many blank lines if the boundary breaks.
    BlankLines(u8),
    /// One blank line if the group breaks, if the boundary breaks.
    BlankIfBroken(u32),
    /// Whitespace chosen by whether a group broke.
    IfBreak {
        group: u32,
        broken: Gap,
        flat: Gap,
    },
    /// The input's whitespace between two tokens, byte for byte: a byte range
    /// of the source, possibly empty.
    Frozen {
        start: u32,
        end: u32,
    },
    /// The start of a group: an index into [`Doc::groups`](super::Doc).
    GroupStart(u32),
    GroupEnd,
    /// The start of an indentation scope.
    ScopeStart(Scope),
    ScopeEnd,
}

/// What a [`Node::Line`] prints as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LineKind {
    /// Nothing in a flat group, a line break in a broken one.
    Soft,
    /// A space in a flat group, a line break in a broken one.
    Normal,
    /// Always a line break.
    Hard,
}

/// How an indentation scope sets the indentation of the lines in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    /// One level more than the enclosing scope.
    Indent,
    /// One level less than the enclosing scope.
    Dedent,
    /// Column 0.
    Root,
    /// The indentation of the line the scope starts on.
    Hug,
}

/// A token printed by the layout.
#[derive(Clone, Copy, Debug)]
pub(super) struct Atom {
    /// The token's index in the document's tokens.
    pub(super) token: u32,
    /// For an [`Place::Inline`] atom, an index into
    /// [`Doc::statics`](super::Doc) when the token is printed as a static
    /// text (a re-cased keyword), else [`NONE`]; for a [`Place::Tag`], the
    /// tag's index in [`Doc::tags`](super::Doc).
    pub(super) aux: u32,
    /// The display width of the printed text's first line, in `char`s.
    pub(super) width: u32,
    /// How the token joins the token before it.
    pub(super) sep: Sep,
    /// Where the token goes.
    pub(super) place: Place,
    /// The printed text spans lines.
    pub(super) multiline: bool,
}

/// How a token joins the token printed before it when the layout puts
/// nothing between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum Sep {
    /// The two may touch.
    Free,
    /// They would lex as other tokens when touching: a space goes between
    /// them if nothing else does.
    Glue,
    /// They touched in the input and one of them is a Jinja expression or a
    /// raw block, which can render as anything: they touch in the output
    /// too, whatever the layout puts between them.
    Touch,
}

/// Where an atom goes on its line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum Place {
    /// Where the layout puts it.
    Inline,
    /// A comment that stays on the line of the code before it, two spaces
    /// after it.
    Trailing,
    /// A comment on a line of its own.
    OwnLine,
    /// A Jinja block tag on a line of its own.
    Tag,
}

/// A run of input tokens printed as written.
#[derive(Clone, Copy, Debug)]
pub(super) struct VerbatimInfo {
    pub(super) first: u32,
    pub(super) last: u32,
    /// Byte for byte, or re-indented as a block.
    pub(super) exact: bool,
    /// On lines of its own, as a block of comments.
    pub(super) own_line: bool,
    /// How the first token joins the token before it.
    pub(super) sep: Sep,
    /// The display width of the block's first line.
    pub(super) first_width: u32,
    /// The block spans lines.
    pub(super) multiline: bool,
}

/// What the printer needs to know about a group before printing it.
#[derive(Clone, Copy, Debug)]
pub(super) struct GroupInfo {
    /// The index of its [`Node::GroupStart`], or [`NONE`] for a group that
    /// was reserved and never built.
    pub(super) start: u32,
    /// The index of its [`Node::GroupEnd`].
    pub(super) end: u32,
    /// Its width when printed flat.
    pub(super) span: Span,
    /// It holds a line break that is always printed, so it never prints flat.
    pub(super) forced: bool,
    /// `span` is its exact flat width, whatever surrounds it: it holds no
    /// frozen whitespace and no [`Node::IfBreak`] that looks at a group
    /// outside it.
    pub(super) exact: bool,
}

/// The width of a stretch of a document printed flat, in a form that
/// composes: what it adds to a line depends only on whether a space or
/// nothing is pending before it, so a group's span is computed once, when
/// it is built, and measuring the group costs no walk over its nodes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Span {
    /// How the first token joins what comes before; `None` if the stretch
    /// holds no token.
    pub(super) first: Option<Sep>,
    /// A layout space comes before the first token (or, without tokens, the
    /// stretch holds a space).
    pub(super) pre_space: bool,
    /// The width from the first token on, the first token's separation
    /// excluded; saturating.
    pub(super) body: u32,
    /// A layout space is pending after the last token.
    pub(super) trail_space: bool,
}

/// What is pending before the next token, for measuring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pend {
    Nothing,
    Space,
    /// The next token starts a line.
    LineStart,
    /// The next token starts the output, unindented: spaces before it are
    /// not printed, but a `.` there gets one (see `Builder::sep`).
    Start,
    /// The input's whitespace is printed before the next token: its width
    /// up to its first line break, and whether it has one.
    Frozen {
        width: u32,
        newline: bool,
    },
}

/// The columns a token's separation takes.
pub(super) fn lead(sep: Sep, pend: Pend) -> u32 {
    match (sep, pend) {
        (Sep::Touch, _)
        | (_, Pend::LineStart | Pend::Frozen { .. })
        | (Sep::Free, Pend::Nothing | Pend::Start) => 0,
        (_, Pend::Space) | (Sep::Glue, Pend::Nothing | Pend::Start) => 1,
    }
}

impl Span {
    pub(super) fn token(&mut self, width: u32, sep: Sep) {
        if self.first.is_some() {
            let pend = if self.trail_space {
                Pend::Space
            } else {
                Pend::Nothing
            };
            self.body = self
                .body
                .saturating_add(lead(sep, pend))
                .saturating_add(width);
        } else {
            self.first = Some(sep);
            self.body = width;
        }
        self.trail_space = false;
    }

    pub(super) fn space(&mut self) {
        if self.first.is_none() {
            self.pre_space = true;
        } else {
            self.trail_space = true;
        }
    }

    pub(super) fn append(&mut self, other: Span) {
        if other.pre_space {
            self.space();
        }
        if let Some(sep) = other.first {
            self.token(other.body, sep);
            self.trail_space = other.trail_space;
        }
    }
}

/// Measures the width of what is printed on a line, against the room left.
#[derive(Clone, Copy, Debug)]
pub(super) struct Meter {
    /// The columns left on the line; negative once it overflows.
    pub(super) room: i64,
    pub(super) pend: Pend,
}

impl Meter {
    /// Adds a token; returns whether the line ends before it, inside frozen
    /// whitespace with a line break.
    pub(super) fn token(&mut self, width: u32, sep: Sep) -> bool {
        if sep != Sep::Touch
            && let Pend::Frozen { width, newline } = self.pend
        {
            self.room -= i64::from(width);
            if newline {
                return true;
            }
        }
        self.room -= i64::from(lead(sep, self.pend)) + i64::from(width);
        self.pend = Pend::Nothing;
        false
    }

    pub(super) fn space(&mut self) {
        if self.pend == Pend::Nothing {
            self.pend = Pend::Space;
        }
    }

    pub(super) fn frozen(&mut self, width: u32, newline: bool) {
        if !matches!(self.pend, Pend::Frozen { .. }) {
            self.pend = Pend::Frozen { width, newline };
        }
    }

    /// Adds a group's flat span; only for a group without frozen
    /// whitespace, so this never ends the line.
    pub(super) fn span(&mut self, span: Span) {
        if span.pre_space {
            self.space();
        }
        if let Some(sep) = span.first {
            self.token(span.body, sep);
            if span.trail_space {
                self.space();
            }
        }
    }

    pub(super) fn fits(&self) -> bool {
        self.room >= 0
    }
}

/// The display width of a text, in `char`s.
pub(super) fn width(text: &str) -> usize {
    if text.is_ascii() {
        text.len()
    } else {
        text.chars().count()
    }
}

/// The display width of a text's first line, and whether it has more lines.
pub(super) fn first_line(text: &str) -> (usize, bool) {
    match text.find('\n') {
        Some(at) => (width(&text[..at]), true),
        None => (width(text), false),
    }
}

/// A width as stored in a document: saturating at `u32::MAX`.
pub(super) fn narrow(width: usize) -> u32 {
    u32::try_from(width).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span_of(items: &[Option<(u32, Sep)>]) -> Span {
        let mut span = Span::default();
        for item in items {
            match *item {
                Some((width, sep)) => span.token(width, sep),
                None => span.space(),
            }
        }
        span
    }

    fn measure(items: &[Option<(u32, Sep)>], pend: Pend) -> (i64, Pend) {
        let mut meter = Meter { room: 100, pend };
        for item in items {
            match *item {
                Some((width, sep)) => {
                    meter.token(width, sep);
                }
                None => meter.space(),
            }
        }
        (100 - meter.room, meter.pend)
    }

    #[test]
    fn spans_compose_like_their_items() {
        use Sep::{Free, Glue, Touch};
        let items = [
            None,
            Some((3, Free)),
            Some((1, Glue)),
            None,
            None,
            Some((2, Touch)),
            Some((4, Glue)),
            None,
            Some((1, Free)),
            None,
        ];
        for pend in [Pend::Nothing, Pend::Space, Pend::LineStart, Pend::Start] {
            let whole = measure(&items, pend);
            for split in 0..=items.len() {
                let mut span = span_of(&items[..split]);
                span.append(span_of(&items[split..]));
                let mut meter = Meter { room: 100, pend };
                meter.span(span);
                assert_eq!(
                    (100 - meter.room, meter.pend),
                    whole,
                    "split at {split}, {pend:?}"
                );
            }
        }
    }

    #[test]
    fn widths_count_chars_and_first_lines() {
        assert_eq!(width("abc"), 3);
        assert_eq!(width("é🙂"), 2);
        assert_eq!(first_line("ab\ncde"), (2, true));
        assert_eq!(first_line("ab"), (2, false));
        assert_eq!(narrow(usize::MAX), u32::MAX);
    }
}
