//! Random documents, driven by bytes, for the printer's property tests and
//! fuzzer.
//!
//! The file is included with `#[path]` by the crate's unit tests, the
//! integration tests and the fuzz target; each provides a module `api` in
//! its parent with the crate's `doc`, `lexer`, `split` and `verify` modules
//! re-exported, so the same code compiles inside and outside the crate.
//!
//! The source text is made of [`WORDS`] joined by [`SPACES`]; the document
//! adds every token of it once, in order, among random scopes and layout.
//! Placement that a real layout takes from the input (whether a comment
//! trails its line, how many blank lines to keep) is taken from the input
//! here too, and every random choice consumes the same bytes whatever the
//! text, so building a document over the printed text with the same bytes
//! makes the same document: that is what the idempotence test relies on.

#![allow(dead_code, reason = "each includer uses part of it")]

use super::api::doc::Builder;
use super::api::doc::Doc;
use super::api::doc::Gap;
use super::api::doc::GroupId;
use super::api::doc::Open;
use super::api::doc::TagId;
use super::api::lexer::Mode;
use super::api::lexer::Token;
use super::api::lexer::TokenKind;
use super::api::split::split;
use super::api::verify::Facts;

/// A source of choices: the bytes, then zeros.
#[derive(Clone)]
pub struct Bytes<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bytes<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Bytes { data, pos: 0 }
    }

    pub fn byte(&mut self) -> u8 {
        let b = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        b
    }

    /// A number below `n` (`n` > 0).
    pub fn below(&mut self, n: usize) -> usize {
        usize::from(self.byte()) % n
    }

    /// True `num` times in 256.
    pub fn chance(&mut self, num: u8) -> bool {
        self.byte() < num
    }

    pub fn exhausted(&self) -> bool {
        self.pos >= self.data.len()
    }
}

/// Words of the source: every kind of token the printer treats specially.
/// No `INSERT` or `FORMAT`, whose data the splitter reads as text.
pub const WORDS: &[&str] = &[
    "select",
    "from",
    "where",
    "and",
    "or",
    "as",
    "null",
    "true",
    "Nan",
    "a",
    "b_1",
    "site",
    "toStartOfHour",
    "count",
    "x",
    "e",
    "1",
    "90",
    "1.5",
    "0x1F",
    "1e5",
    ".5",
    "'s'",
    "'it''s'",
    "x'41'",
    "`q`",
    "\"q\"",
    "$$h$$",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    ",",
    ";",
    ".",
    "::",
    ":",
    "*",
    "/",
    "-",
    "+",
    "=",
    "<",
    ">=",
    "!=",
    "||",
    "->",
    "?",
    "%",
    "-- c",
    "-- note --",
    "# c",
    "/* c */",
    "/* a\n   b */",
    "'multi\n  line'",
    "events_x",
    "$a",
];

/// Extra words in Jinja mode: expressions only, whose spacing the verifier
/// checks without rendering.
pub const JINJA_WORDS: &[&str] = &["{{ x }}", "{{ y | f }}", "events_{{ s }}"];

/// Whitespace between words; never spaces before a line break, so no
/// output line ends in a space even where the input's whitespace is kept.
pub const SPACES: &[&str] = &[
    " ", " ", " ", "  ", "\n", "\n", "\n\n", "\n\n\n", "\t", "\n    ", "\n\t  ", "",
];

/// A source text of `n` words.
pub fn source(bytes: &mut Bytes<'_>, mode: Mode, n: usize) -> String {
    let mut out = String::new();
    let mut line_comment = false;
    for _ in 0..n {
        let jinja = mode == Mode::Jinja && bytes.chance(24);
        let word = if jinja {
            JINJA_WORDS[bytes.below(JINJA_WORDS.len())]
        } else {
            WORDS[bytes.below(WORDS.len())]
        };
        let space = SPACES[bytes.below(SPACES.len())];
        if line_comment && !space.contains('\n') && !out.is_empty() {
            out.push('\n');
        } else if out.is_empty() {
            // Nothing before the first word.
        } else {
            out.push_str(space);
        }
        out.push_str(word);
        line_comment = word.starts_with("--") || word.starts_with('#');
    }
    out
}

/// What the generated document may contain.
#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches, one per kind of node"
)]
pub struct Shape {
    /// Frozen boundaries and frozen tokens.
    pub frozen: bool,
    /// `if_break`s, all monotone: the broken branch breaks wherever the
    /// flat one does.
    pub if_break: bool,
    /// Comments placed as trailing or own-line comments, block tags.
    pub comments: bool,
    /// Verbatim runs of tokens.
    pub verbatim: bool,
    /// Hard lines and blank lines.
    pub hard: bool,
    /// The deepest nesting of scopes.
    pub depth: usize,
}

impl Shape {
    pub const ALL: Shape = Shape {
        frozen: true,
        if_break: true,
        comments: true,
        verbatim: true,
        hard: true,
        depth: 12,
    };
}

/// The tokens of `source`, split as the formatter splits them.
pub fn tokens(source: &str, mode: Mode) -> Vec<Token> {
    split(source, mode).tokens().to_vec()
}

/// A document over `tokens` of `source`.
pub fn document<'s>(
    bytes: &mut Bytes<'_>,
    source: &'s str,
    tokens: &'s [Token],
    mode: Mode,
    shape: Shape,
) -> Doc<'s> {
    let mut b = Builder::new(source, tokens, mode);
    let solid: Vec<usize> = (0..tokens.len())
        .filter(|&i| tokens[i].kind() != TokenKind::Whitespace)
        .collect();
    let mut open: Vec<Open> = Vec::new();
    let mut groups: Vec<GroupId> = Vec::new();
    let mut tags: Vec<TagId> = Vec::new();
    let mut next = 0;
    while next < solid.len() {
        let i = solid[next];
        match bytes.below(16) {
            0 | 1 if open.len() < shape.depth => {
                let scope = match bytes.below(8) {
                    0..=2 => b.open_group(),
                    3 => {
                        let o = b.open_group();
                        b.force_break();
                        o
                    }
                    4 | 5 => b.open_indent(),
                    6 => b.open_hug(),
                    _ if bytes.chance(128) => b.open_dedent(),
                    _ => b.open_at_root(),
                };
                if let Some(group) = scope.group() {
                    groups.push(group);
                }
                open.push(scope);
            }
            2 => {
                if let Some(scope) = open.pop() {
                    b.close(scope);
                }
            }
            3 | 4 => b.line(),
            5 => b.soft_line(),
            6 => b.space(),
            7 if shape.hard => {
                if bytes.chance(128) {
                    b.hard_line();
                } else {
                    b.blank_line();
                }
            }
            8 if shape.if_break && !groups.is_empty() => {
                let group = groups[bytes.below(groups.len())];
                let (broken, flat) = MONOTONE[bytes.below(MONOTONE.len())];
                b.if_break(group, broken, flat);
            }
            9 if shape.frozen => {
                if bytes.chance(128) {
                    b.frozen();
                } else {
                    // The boundary before a token ahead: the whitespace
                    // there, or the two tokens when they touch. Choosing by
                    // token index alone would freeze different boundaries
                    // in a text where the printer added whitespace.
                    let k = (next + bytes.below(3)).min(solid.len() - 1);
                    let right = solid[k];
                    match k.checked_sub(1).map(|j| solid[j]) {
                        Some(left) if right == left + 2 => b.freeze(left + 1),
                        Some(left) => {
                            b.freeze(left);
                            b.freeze(right);
                        }
                        None => b.freeze(right),
                    }
                }
            }
            10 if shape.verbatim => {
                let len = 1 + bytes.below(4);
                let last = solid[(next + len - 1).min(solid.len() - 1)];
                let exact = bytes.chance(64);
                blank_lines(&mut b, i);
                if exact {
                    b.verbatim_exact(i, last);
                } else {
                    b.verbatim(i, last);
                }
                next += len;
            }
            11 if shape.hard && !groups.is_empty() => {
                let group = groups[bytes.below(groups.len())];
                b.blank_line_if_broken(group);
            }
            _ => {
                let kind = tokens[i].kind();
                let as_comment = shape.comments && bytes.chance(200);
                blank_lines(&mut b, i);
                match kind {
                    TokenKind::LineComment | TokenKind::BlockComment if as_comment => {
                        if b.newlines_before(i) == 0 {
                            b.trailing_comment(i);
                        } else {
                            b.own_line_comment(i);
                        }
                    }
                    TokenKind::BareWord if as_comment && tokens[i].text(source) == "where" => {
                        // A stand-in for a block tag: printed on a line of
                        // its own, aligned with the last one.
                        let tag = match tags.last() {
                            Some(&opener) if bytes.chance(128) => b.block_tag_aligned(i, opener),
                            _ => b.block_tag(i),
                        };
                        tags.push(tag);
                    }
                    TokenKind::BareWord if tokens[i].keyword().is_some() && bytes.chance(160) => {
                        if matches!(tokens[i].text(source), "true" | "Nan" | "null") {
                            b.literal_word(i);
                        } else {
                            b.keyword(i);
                        }
                    }
                    _ => b.token(i),
                }
                next += 1;
            }
        }
    }
    while let Some(scope) = open.pop() {
        b.close(scope);
    }
    b.finish()
}

/// Asks for the blank lines the input has before token `i`, at most one.
fn blank_lines(b: &mut Builder<'_>, i: usize) {
    let n = b.newlines_before(i).saturating_sub(1).min(1);
    b.blank_lines(u8::try_from(n).unwrap_or(1));
}

/// `if_break` branches where the broken one breaks wherever the flat one
/// does.
pub const MONOTONE: &[(Gap, Gap)] = &[
    (Gap::Line, Gap::Space),
    (Gap::SoftLine, Gap::Nothing),
    (Gap::Line, Gap::Nothing),
    (Gap::HardLine, Gap::Space),
    (Gap::Space, Gap::Nothing),
    (Gap::Line, Gap::Line),
    (Gap::Nothing, Gap::Nothing),
];

/// The facts the verifier needs about a document's output.
pub fn facts(doc: &Doc<'_>) -> Facts {
    let mut facts = Facts::new();
    for token in doc.recased() {
        facts.recase(token);
    }
    facts
}

/// The lines of `text` that end in a space or a tab.
pub fn trailing_whitespace(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| line.ends_with([' ', '\t']))
        .collect()
}
