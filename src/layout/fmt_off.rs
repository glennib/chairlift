//! The lines between `fmt: off` and `fmt: on` markers, which are kept as
//! written.
//!
//! A marker is a line comment `-- fmt: off` or `-- fmt: on`, spelled as
//! sqlfmt spells it: `--` or `#`, an optional space, `fmt:`, an optional
//! space, `off` or `on`, in any case, and nothing after it but whitespace.
//! In a template, a Jinja comment `{# fmt: off #}` or `{# fmt: on #}` is one
//! too, with any whitespace inside its delimiters. Marker text inside a
//! string or a `/* */` comment is part of that token and is no marker.
//!
//! A region is made of whole lines: from the start of the line the
//! `fmt: off` is on, so a marker that trails code keeps that code's line as
//! well, to the end of the line the matching `fmt: on` is on, or to the end
//! of the text. Markers nest, as in sqlfmt: each `fmt: off` inside a region
//! needs a `fmt: on` of its own before the region ends, and a `fmt: on`
//! outside a region is an ordinary comment.

use crate::lexer::Mode;
use crate::lexer::Token;
use crate::lexer::TokenKind;

/// Adds region `first..=last` to `regions`, joining it to the last one if
/// only whitespace is between them.
fn push(regions: &mut Vec<(usize, usize)>, first: usize, last: usize) {
    match regions.last_mut() {
        Some((_, end)) if first <= *end + 2 => *end = (*end).max(last),
        _ => regions.push((first, last)),
    }
}

/// What a marker asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    Off,
    On,
}

/// The regions of `tokens`, `source`'s lexed in `mode`, to keep as written:
/// the first and last token of each, neither whitespace, in order and apart.
pub(crate) fn regions(source: &str, tokens: &[Token], mode: Mode) -> Vec<(usize, usize)> {
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    for (i, token) in tokens.iter().enumerate() {
        match marker(token, source, mode) {
            Some(Marker::Off) => {
                if depth == 0 {
                    start = line_start(tokens, source, i);
                }
                depth += 1;
            }
            Some(Marker::On) if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    push(&mut regions, start, line_end(tokens, source, i));
                }
            }
            _ => {}
        }
    }
    if depth > 0
        && let Some(last) = tokens
            .iter()
            .rposition(|t| t.kind() != TokenKind::Whitespace)
    {
        push(&mut regions, start, last);
    }
    regions
}

/// The marker `token` is, if it is one.
fn marker(token: &Token, source: &str, mode: Mode) -> Option<Marker> {
    let text = token.text(source);
    match token.kind() {
        TokenKind::LineComment => {
            let rest = text.strip_prefix("--").or_else(|| text.strip_prefix('#'))?;
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            switch(rest.trim_end())
        }
        TokenKind::JinjaComment if mode == Mode::Jinja => {
            let inner = text.strip_prefix("{#")?.strip_suffix("#}")?;
            let inner = inner.strip_prefix(['-', '+']).unwrap_or(inner);
            let inner = inner.strip_suffix(['-', '+']).unwrap_or(inner);
            switch(inner.trim())
        }
        _ => None,
    }
}

/// The marker `text` names: `fmt:`, an optional space, and `off` or `on`,
/// in any case.
fn switch(text: &str) -> Option<Marker> {
    let prefix = text.get(..4)?;
    if !prefix.eq_ignore_ascii_case("fmt:") {
        return None;
    }
    let rest = &text[4..];
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    if rest.eq_ignore_ascii_case("off") {
        Some(Marker::Off)
    } else if rest.eq_ignore_ascii_case("on") {
        Some(Marker::On)
    } else {
        None
    }
}

/// Whether token `i` is whitespace that ends a line.
fn breaks_line(tokens: &[Token], source: &str, i: usize) -> bool {
    tokens[i].kind() == TokenKind::Whitespace && tokens[i].text(source).contains('\n')
}

/// The first token, other than whitespace, of the line token `i` is on.
fn line_start(tokens: &[Token], source: &str, i: usize) -> usize {
    let mut first = i;
    while first > 0 && !breaks_line(tokens, source, first - 1) {
        first -= 1;
    }
    while tokens[first].kind() == TokenKind::Whitespace {
        first += 1;
    }
    first
}

/// The last token, other than whitespace, of the line token `i` is on.
fn line_end(tokens: &[Token], source: &str, i: usize) -> usize {
    let mut last = i;
    while last + 1 < tokens.len() && !breaks_line(tokens, source, last + 1) {
        last += 1;
    }
    while tokens[last].kind() == TokenKind::Whitespace {
        last -= 1;
    }
    last
}

#[cfg(test)]
mod tests;
