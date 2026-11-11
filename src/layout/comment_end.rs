//! Line comments of a template that the rendering can make run on.
//!
//! In SQL a line comment ends at the line break after it, and the layout
//! always keeps a line break there. Once a template renders, the comment
//! ends at the next line break of the rendered text, which is another one
//! where the renderer removes that line break: by a block or comment tag
//! inside the comment whose block ends past it, by a `-` marker on a tag at
//! the comment's end, or by one on the tag after it. The comment then runs
//! on into what the render continues with, up to its next line break, and
//! where the layout puts what follows the comment decides what that is. The
//! verifier checks only the render with every branch taken.
//!
//! What the comment runs into is the same before and after formatting
//! where the render can only continue just after block tags and Jinja
//! comments that end their line without a `-` marker: the layout puts each
//! of those on a line of its own, and keeps a statement it does not lay out
//! on the lines it is written on. So a comment is safe if the tag after it
//! is one of those, as are the tags of its block the render can jump to, and
//! the tags after them where a `-` marker strips their line break in turn.
//! A tag whose rendering holds a line break ends the comment early, inside
//! its own text, which formatting does not change.

use super::writer::TagRole;
use super::writer::tag_name;
use super::writer::tag_role;
use crate::error::CommentEnd;
use crate::lexer::Token;
use crate::lexer::TokenKind;
use crate::lexer::jinja;
use crate::lexer::jinja::TagKind;
use crate::lexer::jinja::WhitespaceControl;

/// The first line comment of `tokens`, a template's, that the rendering can
/// make run on into text formatting may change, and what makes it.
pub(crate) fn comment_ended_by_rendering(
    source: &str,
    tokens: &[Token],
) -> Option<(Token, CommentEnd)> {
    let mut blocks = None;
    for (i, &token) in tokens.iter().enumerate() {
        if token.kind() != TokenKind::LineComment {
            continue;
        }
        let text = token.text(source);
        let tags: Vec<jinja::Tag> = if text.contains('{') {
            jinja::tags(text).collect()
        } else {
            Vec::new()
        };
        // A `-` marker after `# ` can remove the space that makes `#` a
        // comment once rendered, and the layout trims trailing spaces.
        let hash_marker = text.starts_with('#')
            && tags
                .iter()
                .any(|t| t.whitespace_control(text).0 == WhitespaceControl::Strip);
        if hash_marker || tags.iter().any(|t| t.kind != TagKind::Expr) {
            return Some((token, CommentEnd::TagInside));
        }
        let stripped_inside = tags.last().is_some_and(|t| {
            t.whitespace_control(text).1 == WhitespaceControl::Strip
                && text[t.range.end..].trim().is_empty()
        });
        let next = next_solid(tokens, i + 1);
        let stripped_after = next.is_some_and(|n| strips_before(source, tokens[n]));
        if !stripped_inside && !stripped_after {
            continue;
        }
        let blocks = blocks.get_or_insert_with(|| Blocks::new(source, tokens));
        if let Err(reason) = runs_on_safely(source, tokens, blocks, next) {
            let reason = if stripped_inside && reason == CommentEnd::MarkerAfter {
                CommentEnd::MarkerInside
            } else {
                reason
            };
            return Some((token, reason));
        }
    }
    None
}

/// Checks where the render of a comment whose line break is stripped goes
/// on, from token `next` after it: only just after block tags and Jinja
/// comments that end their line, and past those whose `-` marker strips
/// their line break, on to the next ones.
fn runs_on_safely(
    source: &str,
    tokens: &[Token],
    blocks: &Blocks,
    next: Option<usize>,
) -> Result<(), CommentEnd> {
    let mut work: Vec<usize> = next.into_iter().collect();
    let mut seen = Vec::new();
    while let Some(i) = work.pop() {
        if seen.contains(&i) {
            continue;
        }
        seen.push(i);
        let token = tokens[i];
        let resumes: Vec<usize> = match token.kind() {
            TokenKind::JinjaComment => vec![i],
            TokenKind::JinjaBlock => match tag_name(token.text(source)).0 {
                // The render goes on after the tag, or jumps to another of
                // its block.
                "if" | "elif" | "else" | "endif" | "for" | "endfor" | "set" | "endset" => {
                    blocks.members(i).ok_or(CommentEnd::MarkerAfter)?
                }
                // A macro's body renders where the macro is called, not
                // here: the render goes on after `{% endmacro %}`.
                "macro" => blocks
                    .members(i)
                    .and_then(|m| m.last().copied().filter(|&c| c != i))
                    .map(|c| vec![c])
                    .ok_or(CommentEnd::MarkerAfter)?,
                "endmacro" => return Err(CommentEnd::MacroBody),
                _ => return Err(CommentEnd::MarkerAfter),
            },
            _ => return Err(CommentEnd::MarkerAfter),
        };
        for r in resumes {
            let text = tokens[r].text(source);
            let strips_after = jinja::tags(text)
                .next()
                .is_some_and(|t| t.whitespace_control(text).1 == WhitespaceControl::Strip);
            let after = next_solid(tokens, r + 1);
            if strips_after {
                work.extend(after);
                continue;
            }
            let ends_line = tokens
                .get(r + 1)
                .is_none_or(|t| t.kind() == TokenKind::Whitespace && t.text(source).contains('\n'));
            if !ends_line {
                return Err(CommentEnd::MarkerAfter);
            }
            if let Some(after) = after
                && strips_before(source, tokens[after])
            {
                work.push(after);
            }
        }
    }
    Ok(())
}

/// The index of the first token from `from` on that is not whitespace.
fn next_solid(tokens: &[Token], from: usize) -> Option<usize> {
    (from..tokens.len()).find(|&i| tokens[i].kind() != TokenKind::Whitespace)
}

/// Whether `token` starts with a tag whose `-` marker strips the whitespace
/// before it.
fn strips_before(source: &str, token: Token) -> bool {
    let text = token.text(source);
    jinja::tags(text).next().is_some_and(|tag| {
        tag.range.start == 0 && tag.whitespace_control(text).0 == WhitespaceControl::Strip
    })
}

/// The blocks of a template, as the renderer nests them: every block tag
/// counts, also one inside a SQL string or comment.
struct Blocks {
    /// For each token that is a block tag of its own, its block.
    block_of: Vec<Option<usize>>,
    /// The tokens of each block's tags, in order; `None` where one of them
    /// is inside another token, after which text can follow on its line.
    blocks: Vec<Option<Vec<usize>>>,
}

impl Blocks {
    fn new(source: &str, tokens: &[Token]) -> Blocks {
        let mut block_of = vec![None; tokens.len()];
        let mut blocks: Vec<Option<Vec<usize>>> = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        for (i, token) in tokens.iter().enumerate() {
            let text = token.text(source);
            if !text.contains("{%") {
                continue;
            }
            let alone = token.kind() == TokenKind::JinjaBlock;
            for tag in jinja::tags(text).filter(|t| t.kind == TagKind::Block) {
                let role = tag_role(&text[tag.range]);
                let block = match role {
                    TagRole::Open | TagRole::Other => None,
                    TagRole::Middle => open.last().copied(),
                    TagRole::Close => open.pop(),
                };
                let block = block.unwrap_or_else(|| {
                    blocks.push(Some(Vec::new()));
                    blocks.len() - 1
                });
                if role == TagRole::Open {
                    open.push(block);
                }
                if alone {
                    block_of[i] = Some(block);
                }
                match (&mut blocks[block], alone) {
                    (Some(members), true) => members.push(i),
                    (members, false) => *members = None,
                    (None, true) => {}
                }
            }
        }
        Blocks { block_of, blocks }
    }

    /// The tokens of the tags of the block of token `i`, a block tag of its
    /// own, if each is one.
    fn members(&self, i: usize) -> Option<Vec<usize>> {
        self.block_of[i].and_then(|b| self.blocks[b].clone())
    }
}
