//! Jinja block and comment tags inside a string, quoted identifier or block
//! comment, which the renderer acts on although the SQL sees one token.
//!
//! The layout cannot see the branches of such a tag: a block that opens in
//! one token and closes in another, or outside any, decides what renders
//! between them. A token whose tags are [contained](contained) in it is
//! different: its blocks open and close inside it, so every render changes
//! only its own text, and every render of that text is still one token of
//! its kind, so it ends where it ends in the template. Formatting never
//! changes a token's text, so such a token is no obstacle to laying out the
//! text around it.

use std::ops::Range;

use super::writer::TagRole;
use super::writer::tag_name;
use super::writer::tag_role;
use crate::lexer::Mode;
use crate::lexer::TokenKind;
use crate::lexer::jinja;
use crate::lexer::jinja::TagKind;

/// The most renders of one token's text that are tried; a token with more
/// counts as not contained.
const MAX_RENDERS: usize = 64;

/// How deep blocks may nest inside one token.
const MAX_DEPTH: usize = 8;

/// Whether the block and comment tags in `text`, a token of `kind`, are
/// contained in it: `kind` is a string, a quoted identifier or a block
/// comment, every block in it opens and closes in it and is an `if` or a
/// `for`, and every render of it, with each branch taken or left out and
/// each loop run zero, one or two times, lexes as one token of `kind`.
pub(crate) fn contained(text: &str, kind: TokenKind) -> bool {
    if !matches!(
        kind,
        TokenKind::StringLiteral | TokenKind::QuotedIdentifier | TokenKind::BlockComment
    ) {
        return false;
    }
    let tags: Vec<jinja::Tag> = jinja::tags(text).collect();
    if tags.iter().any(|t| !t.closed || t.kind == TagKind::Raw) {
        return false;
    }
    let mut parser = Blocks {
        text,
        tags: &tags,
        next: 0,
    };
    let Some(items) = parser.sequence(0) else {
        return false;
    };
    if parser.next < tags.len() {
        return false;
    }
    let Some(renders) = renders(text, &items) else {
        return false;
    };
    renders.iter().all(|render| {
        let tokens = crate::lexer::tokenize(render, Mode::Sql);
        tokens.len() == 1 && tokens[0].kind() == kind
    })
}

/// A piece of a token's text, as the renderer sees it.
enum Item {
    /// Text that renders as it is: a range of the token's text.
    Text(Range<usize>),
    /// An `if` chain: the items of each branch, and whether one of them is
    /// an `else`.
    If(Vec<Vec<Item>>, bool),
    /// A `for` loop's body.
    For(Vec<Item>),
}

/// Reads the tags of a token's text into [`Item`]s.
struct Blocks<'a> {
    text: &'a str,
    tags: &'a [jinja::Tag],
    /// The next tag to read.
    next: usize,
}

impl Blocks<'_> {
    /// Items up to the end of the text or the next tag that continues or
    /// closes the enclosing block, which is left unread; `None` for a block
    /// of another kind, nested too deep.
    fn sequence(&mut self, depth: usize) -> Option<Vec<Item>> {
        if depth > MAX_DEPTH {
            return None;
        }
        let mut items = Vec::new();
        let mut pos = self
            .tags
            .get(self.next.wrapping_sub(1))
            .map_or(0, |t| t.range.end);
        while let Some(tag) = self.tags.get(self.next) {
            items.push(Item::Text(pos..tag.range.start));
            let tag_text = &self.text[tag.range.clone()];
            match tag.kind {
                TagKind::Expr => {
                    items.push(Item::Text(tag.range.clone()));
                    self.next += 1;
                }
                TagKind::Comment => self.next += 1,
                TagKind::Raw => return None,
                TagKind::Block => match (tag_name(tag_text).0, tag_role(tag_text)) {
                    ("if", _) => {
                        self.next += 1;
                        let mut branches = vec![self.sequence(depth + 1)?];
                        let mut has_else = false;
                        loop {
                            let tag = self.tags.get(self.next)?;
                            let name = tag_name(&self.text[tag.range.clone()]).0;
                            self.next += 1;
                            match name {
                                "elif" if !has_else => {}
                                "else" if !has_else => has_else = true,
                                "endif" => break,
                                _ => return None,
                            }
                            branches.push(self.sequence(depth + 1)?);
                        }
                        items.push(Item::If(branches, has_else));
                    }
                    ("for", _) => {
                        self.next += 1;
                        let body = self.sequence(depth + 1)?;
                        let tag = self.tags.get(self.next)?;
                        if tag_name(&self.text[tag.range.clone()]).0 != "endfor" {
                            return None;
                        }
                        self.next += 1;
                        items.push(Item::For(body));
                    }
                    ("set", TagRole::Other) => self.next += 1,
                    (_, TagRole::Middle | TagRole::Close) if depth > 0 => return Some(items),
                    _ => return None,
                },
            }
            pos = self.tags[self.next - 1].range.end;
        }
        items.push(Item::Text(pos..self.text.len()));
        Some(items)
    }
}

/// Every text `items` can render as, at most [`MAX_RENDERS`].
fn renders(text: &str, items: &[Item]) -> Option<Vec<String>> {
    let mut out = vec![String::new()];
    for item in items {
        let options: Vec<String> = match item {
            Item::Text(range) => vec![text[range.clone()].to_owned()],
            Item::If(branches, has_else) => {
                let mut options = Vec::new();
                for branch in branches {
                    options.extend(renders(text, branch)?);
                }
                if !has_else {
                    options.push(String::new());
                }
                options
            }
            Item::For(body) => {
                let once = renders(text, body)?;
                let mut options = vec![String::new()];
                options.extend(once.iter().cloned());
                for a in &once {
                    for b in &once {
                        options.push(format!("{a}{b}"));
                    }
                }
                options
            }
        };
        if out.len().saturating_mul(options.len()) > MAX_RENDERS {
            return None;
        }
        out = out
            .iter()
            .flat_map(|prefix| options.iter().map(move |o| format!("{prefix}{o}")))
            .collect();
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_inside_a_string_are_contained() {
        let s = TokenKind::StringLiteral;
        assert!(contained("'{% if a %}16Mi{% else %}0{% endif %}'", s));
        assert!(contained("'{% for x in xs %}{{ x }},{% endfor %}'", s));
        assert!(contained(
            "/* {# note #} {% if a %}x{% endif %} */",
            TokenKind::BlockComment
        ));
    }

    #[test]
    fn blocks_that_reach_outside_or_change_the_token_are_not() {
        let s = TokenKind::StringLiteral;
        // Opened here, closed elsewhere.
        assert!(!contained("'{% if a %}x'", s));
        assert!(!contained("'x{% endif %}'", s));
        // A render ends the string early, or escapes its closing quote.
        assert!(!contained("'a{% if x %}'{% endif %}'", s));
        assert!(!contained("'a\\{% if x %}{% endif %}'", s));
        // A word with a tag glued into it, and a macro.
        assert!(!contained("k{% if a %}", TokenKind::BareWord));
        assert!(!contained("'{% macro m() %}x{% endmacro %}'", s));
    }
}
