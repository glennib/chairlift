//! Jinja tags: `{{ … }}`, `{% … %}`, `{# … #}` and `{% raw %}…{% endraw %}`.
//!
//! Tags are found the way minijinja's tokenizer finds them, with the default
//! syntax and no line statements, so the lexer agrees with the renderer about
//! where every tag starts and ends:
//!
//! - a tag starts at `{` followed by `{`, `%` or `#`, anywhere, including
//!   inside SQL strings and comments (the renderer knows nothing about SQL);
//!   a `-` or `+` right after the opener is a whitespace-control marker;
//! - inside `{{ … }}` and `{% … %}`, strings in `'…'` or `"…"` (with backslash
//!   escapes) and brackets are skipped, so `{{ "%}" }}` and
//!   `{{ {'a': 1} }}` end at their last delimiter, and the closer (optionally
//!   preceded by `-` or `+`) only counts outside brackets;
//! - `{# … #}` ends at the first `#}`;
//! - `{% raw %}` (the tag name alone, with optional markers) starts a raw block
//!   that ends at the first `{% endraw %}`; nothing inside is a tag.
//!
//! A tag that is not closed runs to the end of the input.

use std::ops::Range;

/// What a tag is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TagKind {
    /// `{{ … }}`
    Expr,
    /// `{% … %}`, other than a raw block.
    Block,
    /// `{# … #}`
    Comment,
    /// `{% raw %}…{% endraw %}`, the whole block.
    Raw,
}

/// A tag within a text.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Tag {
    /// What the tag is.
    pub kind: TagKind,
    /// Its byte range in the text it was found in.
    pub range: Range<usize>,
    /// Whether the tag has its closer; an unclosed tag runs to the end of the
    /// text.
    pub closed: bool,
}

/// What a tag's whitespace-control marker asks the renderer to do with the
/// whitespace on that side of the tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WhitespaceControl {
    /// No marker: the environment's settings apply (chum's keep the
    /// whitespace).
    Default,
    /// `-`: the whitespace next to the tag is removed, including newlines.
    Strip,
    /// `+`: the whitespace is kept even where `lstrip_blocks` or `trim_blocks`
    /// would remove it.
    Preserve,
}

impl Tag {
    /// The markers on the tag's two sides, as `(before, after)`; `text` is the
    /// text the tag was found in.
    ///
    /// ```
    /// use chairlift::lexer::jinja::{WhitespaceControl, tags};
    ///
    /// let text = "{%- if x +%}";
    /// let tag = tags(text).next().unwrap();
    /// assert_eq!(
    ///     tag.whitespace_control(text),
    ///     (WhitespaceControl::Strip, WhitespaceControl::Preserve)
    /// );
    /// ```
    #[must_use]
    pub fn whitespace_control(&self, text: &str) -> (WhitespaceControl, WhitespaceControl) {
        let tag = &text.as_bytes()[self.range.clone()];
        let control = |b: Option<&u8>| match b {
            Some(b'-') => WhitespaceControl::Strip,
            Some(b'+') => WhitespaceControl::Preserve,
            _ => WhitespaceControl::Default,
        };
        let before = control(tag.get(2));
        let opener_len = 2 + usize::from(before != WhitespaceControl::Default);
        // The byte before the two-byte closer, unless it belongs to the opener
        // (`{{-}}` has only a leading marker).
        let after = if self.closed && tag.len() >= opener_len + 3 {
            control(tag.get(tag.len() - 3))
        } else {
            WhitespaceControl::Default
        };
        (before, after)
    }
}

/// The tags in `text`, in order.
///
/// For a token lexed in [Jinja mode](super::Mode::Jinja), these are exactly
/// the tags the lexer found in it: the tag of a
/// [`JinjaExpr`](super::TokenKind::JinjaExpr), the tags glued into a
/// [`BareWord`](super::TokenKind::BareWord), or tags inside a string or a
/// comment.
///
/// ```
/// use chairlift::lexer::jinja::{TagKind, tags};
///
/// let found: Vec<_> = tags("'{{ a }}_{# b #}'").map(|t| (t.kind, t.range)).collect();
/// assert_eq!(found, [(TagKind::Expr, 1..8), (TagKind::Comment, 9..16)]);
/// ```
#[must_use]
pub fn tags(text: &str) -> Tags<'_> {
    Tags {
        bytes: text.as_bytes(),
        pos: 0,
    }
}

/// The iterator [`tags`] returns.
#[derive(Clone, Debug)]
pub struct Tags<'t> {
    bytes: &'t [u8],
    pos: usize,
}

impl Iterator for Tags<'_> {
    type Item = Tag;

    fn next(&mut self) -> Option<Tag> {
        let b = self.bytes;
        while self.pos < b.len() {
            let start = self.pos;
            if is_tag_start(b, start) {
                let scanned = scan_tag(b, start);
                self.pos = scanned.end;
                return Some(Tag {
                    kind: scanned.kind,
                    range: start..scanned.end,
                    closed: scanned.closed,
                });
            }
            self.pos += 1;
        }
        None
    }
}

impl std::iter::FusedIterator for Tags<'_> {}

/// A tag found by [`scan_tag`].
pub(super) struct Scanned {
    pub(super) end: usize,
    pub(super) kind: TagKind,
    pub(super) closed: bool,
}

/// Whether a tag starts at `pos`.
#[inline]
pub(super) fn is_tag_start(b: &[u8], pos: usize) -> bool {
    b[pos] == b'{' && matches!(b.get(pos + 1), Some(b'{' | b'%' | b'#'))
}

/// Scans the tag that starts at `start`, which [`is_tag_start`].
pub(super) fn scan_tag(b: &[u8], start: usize) -> Scanned {
    let marker = b[start + 1];
    let body = start + 2 + usize::from(matches!(b.get(start + 2), Some(b'-' | b'+')));
    let unclosed = |kind| Scanned {
        end: b.len(),
        kind,
        closed: false,
    };
    match marker {
        b'#' => match find(b, body, b"#}") {
            Some(pos) => Scanned {
                end: pos + 2,
                kind: TagKind::Comment,
                closed: true,
            },
            None => unclosed(TagKind::Comment),
        },
        b'%' => {
            if let Some(mut pos) = simple_tag(b, body, b"raw", false) {
                while let Some(open) = find(b, pos, b"{%") {
                    pos = open + 2;
                    if let Some(end) = simple_tag(b, pos, b"endraw", true) {
                        return Scanned {
                            end,
                            kind: TagKind::Raw,
                            closed: true,
                        };
                    }
                }
                unclosed(TagKind::Raw)
            } else {
                expression(b, body, b"%}", TagKind::Block)
            }
        }
        _ => expression(b, body, b"}}", TagKind::Expr),
    }
}

/// The interior of `{{ … }}` or `{% … %}` from `pos`, up to `close`.
fn expression(b: &[u8], mut pos: usize, close: &[u8], kind: TagKind) -> Scanned {
    let mut depth: isize = 0;
    while pos < b.len() {
        let c = b[pos];
        if depth == 0 {
            if matches!(c, b'-' | b'+') && b[pos + 1..].starts_with(close) {
                return closed(pos + 1 + close.len(), kind);
            }
            if b[pos..].starts_with(close) {
                return closed(pos + close.len(), kind);
            }
        }
        match c {
            b'\'' | b'"' => {
                pos += 1;
                loop {
                    match b.get(pos) {
                        None => {
                            return Scanned {
                                end: b.len(),
                                kind,
                                closed: false,
                            };
                        }
                        Some(b'\\') => pos += 2,
                        Some(&d) if d == c => break,
                        Some(_) => pos += 1,
                    }
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        pos += 1;
    }
    Scanned {
        end: b.len(),
        kind,
        closed: false,
    }
}

fn closed(end: usize, kind: TagKind) -> Scanned {
    Scanned {
        end,
        kind,
        closed: true,
    }
}

/// minijinja's `skip_basic_tag`: whether the text at `pos` is the rest of a
/// tag consisting of `name` alone, as in `raw %}` or `- endraw -%}`; returns
/// the tag's end.
fn simple_tag(b: &[u8], mut pos: usize, name: &[u8], leading_marker: bool) -> Option<usize> {
    let skip_whitespace = |mut pos: usize| {
        while b.get(pos).is_some_and(u8::is_ascii_whitespace) {
            pos += 1;
        }
        pos
    };
    if leading_marker && matches!(b.get(pos), Some(b'-' | b'+')) {
        pos += 1;
    }
    pos = skip_whitespace(pos);
    if !b[pos..].starts_with(name) {
        return None;
    }
    pos = skip_whitespace(pos + name.len());
    if matches!(b.get(pos), Some(b'-' | b'+')) {
        pos += 1;
    }
    b[pos..].starts_with(b"%}").then_some(pos + 2)
}

/// The position of the first `needle` at or after `from`.
fn find(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    b.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| from + i)
}
