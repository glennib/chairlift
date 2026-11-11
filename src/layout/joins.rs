//! Words of a template that a render can join to a token on the other side
//! of a block tag, which keep their case.
//!
//! A block tag renders nothing, and a `-` marker strips the whitespace next
//! to its tag, so in a render a word before `{%- if x %}` meets whatever the
//! render goes on with after the tag. The verifier checks the render with
//! every branch taken and every loop body once, where that is what follows
//! the tag. In other renders the render goes on after another tag of the
//! block: a branch left out, a loop run twice or not at all. If the word
//! meets a token there that it does not stay apart from, the two are one
//! token in that render, `endand` in `end {%- if y %} … {% else -%} and`,
//! and re-casing the word would change it. The same holds for a word after a
//! tag whose `-` marker strips the whitespace before it, and the token the
//! render can put before it.

use super::comment_end::Blocks;
use super::comment_end::next_solid;
use super::comment_end::strips_before;
use super::writer::tag_name;
use crate::lexer::Mode;
use crate::lexer::Token;
use crate::lexer::TokenKind;
use crate::lexer::can_touch;
use crate::lexer::jinja;
use crate::lexer::jinja::WhitespaceControl;

/// The most tags one block may have for the search to follow it; a word
/// next to a bigger one keeps its case.
const MAX_MEMBERS: usize = 64;

/// For each token of a template, whether it is a word that a render can join
/// to a token beyond a block tag next to it; all `false` for a text without
/// a block or comment tag that strips or touches.
pub(crate) fn joinable(source: &str, tokens: &[Token]) -> Vec<bool> {
    let mut out = vec![false; tokens.len()];
    if !(source.contains("{%") || source.contains("{#")) {
        return out;
    }
    let mut blocks = None;
    for (i, token) in tokens.iter().enumerate() {
        if token.kind() != TokenKind::BareWord {
            continue;
        }
        let after = next_solid(tokens, i + 1)
            .filter(|&k| is_tag(tokens[k]) && (k == i + 1 || strips_before(source, tokens[k])));
        let before = previous_solid(tokens, i)
            .filter(|&k| is_tag(tokens[k]) && (k + 1 == i || strips_after(source, tokens[k])));
        if after.is_none() && before.is_none() {
            continue;
        }
        let blocks = blocks.get_or_insert_with(|| Blocks::new(source, tokens));
        let search = Search {
            source,
            tokens,
            blocks,
            word: i,
        };
        out[i] = after.is_some_and(|k| search.meets(k, Side::After))
            || before.is_some_and(|k| search.meets(k, Side::Before));
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    /// The tags follow the word.
    After,
    /// The tags come before it.
    Before,
}

struct Search<'a> {
    source: &'a str,
    tokens: &'a [Token],
    blocks: &'a Blocks,
    word: usize,
}

impl Search<'_> {
    /// Whether, past the tag `tag` on `side` of the word, a render can put
    /// a token against the word that it does not stay apart from.
    fn meets(&self, tag: usize, side: Side) -> bool {
        let mut pending = vec![tag];
        let mut seen = Vec::new();
        while let Some(t) = pending.pop() {
            if seen.contains(&t) {
                continue;
            }
            seen.push(t);
            if seen.len() > MAX_MEMBERS {
                return true;
            }
            let Some(points) = self.points(t, side) else {
                return true;
            };
            for r in points {
                let (next, strips) = match side {
                    Side::After => (next_solid(self.tokens, r + 1), self.strips_after(r)),
                    Side::Before => (previous_solid(self.tokens, r), self.strips_before(r)),
                };
                let Some(n) = next else {
                    continue;
                };
                let spaced = n.abs_diff(r) > 1;
                if spaced && !strips {
                    continue;
                }
                if is_tag(self.tokens[n]) {
                    pending.push(n);
                    continue;
                }
                let word = self.tokens[self.word];
                let (word, other) = (
                    (word.kind(), word.text(self.source)),
                    (self.tokens[n].kind(), self.tokens[n].text(self.source)),
                );
                let apart = match side {
                    Side::After => can_touch(Mode::Sql, None, word, other),
                    Side::Before => can_touch(Mode::Sql, None, other, word),
                };
                if !apart {
                    return true;
                }
            }
        }
        false
    }

    /// The tags past which a render that reaches tag `t` from `side` can go
    /// on: the tag itself for a Jinja comment and a tag that opens no block,
    /// every tag of its block for an `if` or a `for`; `None` where the render
    /// can go elsewhere, such as from the end of a macro's body to wherever
    /// it is called.
    fn points(&self, t: usize, side: Side) -> Option<Vec<usize>> {
        let token = self.tokens[t];
        if token.kind() == TokenKind::JinjaComment {
            return Some(vec![t]);
        }
        match tag_name(token.text(self.source)).0 {
            "if" | "elif" | "else" | "endif" | "for" | "endfor" | "set" | "endset" => {
                let members = self.blocks.members(t)?;
                (members.len() <= MAX_MEMBERS).then_some(members)
            }
            "macro" if side == Side::After => {
                let members = self.blocks.members(t)?;
                (members.len() <= MAX_MEMBERS).then_some(members)
            }
            "do" => Some(vec![t]),
            _ => None,
        }
    }

    fn strips_after(&self, i: usize) -> bool {
        strips_after(self.source, self.tokens[i])
    }

    fn strips_before(&self, i: usize) -> bool {
        strips_before(self.source, self.tokens[i])
    }
}

fn is_tag(token: Token) -> bool {
    matches!(
        token.kind(),
        TokenKind::JinjaBlock | TokenKind::JinjaComment
    )
}

/// The index of the last token before `before` that is not whitespace.
fn previous_solid(tokens: &[Token], before: usize) -> Option<usize> {
    (0..before)
        .rev()
        .find(|&i| tokens[i].kind() != TokenKind::Whitespace)
}

/// Whether `token` is a tag whose `-` marker strips the whitespace after it.
fn strips_after(source: &str, token: Token) -> bool {
    let text = token.text(source);
    jinja::tags(text).next().is_some_and(|tag| {
        tag.range.end == text.len() && tag.whitespace_control(text).1 == WhitespaceControl::Strip
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(source: &str) -> Vec<&str> {
        let tokens = crate::lexer::tokenize(source, Mode::Jinja);
        joinable(source, &tokens)
            .iter()
            .zip(&tokens)
            .filter(|(j, _)| **j)
            .map(|(_, t)| t.text(source))
            .collect()
    }

    #[test]
    fn words_a_left_out_branch_joins_keep_their_case() {
        assert_eq!(
            joined("end {%- if y %} x and b {% else -%} and b {% endif %}"),
            ["end", "and"]
        );
        // A loop run twice joins the end of its body to its start.
        assert_eq!(joined("{% for i in xs -%} a {%- endfor %} b"), ["a"]);
    }

    #[test]
    fn words_with_whitespace_in_every_render_are_free() {
        let none: [&str; 0] = [];
        // The proposal's example: the whitespace before `{% if` stays.
        assert_eq!(
            joined(
                "where true\n    and t = 'p'\n    {% if c -%}\n    and e >= 1\n    {% endif -%}\ngroup by h"
            ),
            none
        );
        assert_eq!(joined("a {%- if x %} b {% endif %} c"), none);
        // Punctuation stays apart from a word.
        assert_eq!(joined("end {%- if y %} x {% else -%}, b {% endif %}"), none);
    }

    #[test]
    fn a_macro_body_ends_where_it_is_called() {
        assert_eq!(joined("{% macro m() -%} a {%- endmacro %}"), ["a"]);
    }
}
