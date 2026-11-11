//! The document IR and the width-aware printer.
//!
//! A layout rule turns the syntax tree into a [`Doc`]: the text's tokens, in
//! order, with the whitespace between them described by layout nodes
//! (spaces, places a line may break, groups that break together,
//! indentation). The [printer](Doc::print) chooses the line breaks for a line
//! width and writes the text. The IR is Wadler's and Prettier's, with the
//! additions SQL with comments and Jinja needs; the [`pretty`] crate's
//! combinators map onto it, with lazy whitespace, line comments and frozen
//! whitespace as the parts it lacks.
//!
//! A document refers to tokens by their index in the token slice it was
//! built over (the formatter's [`Split::tokens`](crate::split::Split::tokens),
//! which the syntax tree holds one for one) and to text by byte range, so it
//! holds no copy of any token: building one allocates a few vectors, never a
//! `String` per token. Nodes are a flat vector, groups and scopes are start
//! and end markers, and printing walks the vector once, without recursion,
//! however deep the nesting.
//!
//! [`pretty`]: https://docs.rs/pretty
//!
//! ```
//! use chairlift::doc::{Builder, PrintOptions};
//! use chairlift::lexer::{Mode, tokenize};
//!
//! let source = "where a = 1 and b = 2";
//! let tokens = tokenize(source, Mode::Sql);
//! let doc = {
//!     let mut b = Builder::new(source, &tokens, Mode::Sql);
//!     b.keyword(0); // where
//!     b.space();
//!     b.group(|b| {
//!         for i in [2, 4, 6] {
//!             b.token(i); // a = 1
//!             if i < 6 {
//!                 b.space();
//!             }
//!         }
//!         b.indent(|b| {
//!             b.line();
//!             b.keyword(8); // and
//!             for i in [10, 12, 14] {
//!                 b.space();
//!                 b.token(i); // b = 2
//!             }
//!         });
//!     });
//!     b.finish()
//! };
//! doc.check_tokens()?;
//! let print = |width| doc.print(&PrintOptions::default().with_line_width(width));
//! assert_eq!(print(100), "WHERE a = 1 AND b = 2\n");
//! assert_eq!(print(15), "WHERE a = 1\n    AND b = 2\n");
//! # Ok::<(), chairlift::doc::TokenCheckError>(())
//! ```
//!
//! # Vocabulary
//!
//! Written below as `group(a line b)` for the [`Builder`] calls
//! `group(|b| { token(a); line(); token(b) })`; `|` separates what the
//! printer chooses between.
//!
//! **Tokens.** Every token of the source other than whitespace is added
//! exactly once, in order.
//!
//! - [`token`](Builder::token): the token as written.
//! - [`keyword`](Builder::keyword), [`literal_word`](Builder::literal_word),
//!   [`recased`](Builder::recased): the token in another case, from a static
//!   text (`select` → `SELECT`, `Nan` → `nan`), so re-casing allocates
//!   nothing; [`Doc::recased`] lists them for the verifier.
//! - [`trailing_comment`](Builder::trailing_comment): `a,  -- c` — stays on
//!   the line of the code before it, two spaces after it, even across a
//!   line break the layout puts before it; the next token starts a line.
//! - [`own_line_comment`](Builder::own_line_comment): a line of its own at
//!   the scope's indentation; a multi-line `/* */` keeps its bytes.
//! - [`comment_block`](Builder::comment_block): several comment lines,
//!   re-indented as a block, keeping their indentation relative to each
//!   other.
//! - [`block_tag`](Builder::block_tag) and
//!   [`block_tag_aligned`](Builder::block_tag_aligned): `{% if %}` on a line
//!   of its own; `{% endif %}` at the indentation its opener was printed at,
//!   wherever the layout puts it.
//! - [`verbatim`](Builder::verbatim): a run of tokens as written,
//!   re-indented as a block, for what the parser kept verbatim.
//! - [`verbatim_exact`](Builder::verbatim_exact): a run of tokens byte for
//!   byte, for text ClickHouse reads as written (the left operand of `::`).
//!
//! **Whitespace.** Printed lazily, when the next token is.
//!
//! - [`space`](Builder::space): ` `.
//! - [`line`](Builder::line): ` ` | line break; [`soft_line`](Builder::soft_line):
//!   nothing | line break — chosen by the innermost group.
//! - [`hard_line`](Builder::hard_line): a line break; every group around it
//!   breaks.
//! - [`blank_lines(n)`](Builder::blank_lines): up to `n` blank lines here, if
//!   a line breaks here; [`blank_line`](Builder::blank_line): a line break and
//!   a blank line; [`blank_line_if_broken(g)`](Builder::blank_line_if_broken):
//!   a blank line if group `g` breaks, even one built after this place.
//! - [`if_break(g, broken, flat)`](Builder::if_break): whitespace chosen by
//!   group `g`, as `GROUP BY k WITH TOTALS` uses it. There is no conditional
//!   text: a token printed only when a group breaks (a trailing comma) would
//!   be added or dropped, which chairlift never does.
//! - [`frozen`](Builder::frozen) and [`freeze`](Builder::freeze): the
//!   input's whitespace at a place, byte for byte, in place of the layout's.
//!
//! **Scopes.**
//!
//! - [`group`](Builder::group): `group(a line b)` prints `a b` if that fits
//!   on the line up to the next place it can break, else `a⏎b`; a group in a
//!   flat group is flat. [`group_forced`](Builder::group_forced) and
//!   [`force_break`](Builder::force_break) never print flat.
//! - [`indent`](Builder::indent), [`dedent`](Builder::dedent),
//!   [`at_root`](Builder::at_root): the indentation of lines broken in the
//!   scope; [`hug`](Builder::hug): the indentation of the line the scope
//!   starts on, so `FROM (⏎    SELECT …⏎) AS d` closes where its line
//!   starts.
//!
//! # What the printer guarantees
//!
//! - **Width.** A group prints flat only if it fits, with what follows it up
//!   to the next place a line can break (every group after it taken as
//!   broken): Wadler's and Prettier's measure. Each group is measured once,
//!   when the printer reaches it inside a broken group; a group's flat width
//!   is computed when it is built, so measuring costs a walk over a group's
//!   own nodes only where it looks at other groups or holds frozen
//!   whitespace. Trailing comments are not measured. Where every boundary
//!   can break, no line is wider than the width unless one token is.
//! - **Breaks.** A group with a hard line, a line comment, an own-line
//!   comment, a block tag or a token that spans lines never prints flat, nor
//!   does any group around it. Line breaks at one place do not add up.
//! - **Indentation.** A line break is indented as the scope it is asked for
//!   in; where several meet, the last one wins. After a line comment, an
//!   own-line comment or a block tag, the next token starts a line indented
//!   as the scope of the next token or group, unless the layout breaks the
//!   line there itself.
//! - **Whitespace.** No line ends in whitespace, except inside a token,
//!   frozen whitespace, exact verbatim text and a `# ` comment (whose space
//!   makes it one). Token text is never trimmed; a line comment loses only
//!   the whitespace at its end, which is not its text to the verifier.
//! - **Tokens.** Two tokens with nothing between them touch only if
//!   [`can_touch`](crate::lexer::can_touch) allows it; otherwise a space goes
//!   between them, so `-` `-1` prints as `- -1`, never `--1`. The layout
//!   says "no space" (`f(`, `a,`, `t.c`, `x::T`) and the printer keeps it
//!   safe. Next to a Jinja expression or raw block, whitespace is present
//!   exactly where it is in the input, whatever the layout says.
//! - **Blank lines.** At most two in a row, none right after `(` or right
//!   before `)`, none at the start of the output.
//! - **Start and end.** The output starts without a blank line, with the
//!   source's byte-order mark if it has one, and ends as [`Ending`] says.
//! - **Exactly once.** [`Doc::check_tokens`] checks that the document
//!   prints every token other than whitespace once, in order; the printer
//!   prints each token node exactly once by construction.
//!
//! # Style rules and their constructs
//!
//! | Rule (`docs/MILESTONES.md`, "Style") | Construct |
//! | --- | --- |
//! | Line width 100, four-space indent | [`PrintOptions`] |
//! | Keywords upper case; `true`, `nan` lower, `NULL` upper | `keyword`, `literal_word` |
//! | A list fits on its line, else one item per line | `group(indent(soft_line a , line b) soft_line)` |
//! | A `SELECT` list of several items always splits | `group_forced(SELECT indent(line a , line b))` |
//! | Hanging conditions, leading `AND`/`OR`; `ON`, `HAVING` alike | `WHERE group(c1 indent(line AND c2 line AND c3))` |
//! | Subquery, single CTE, projection hug `(` | `( hug(indent(hard_line query) hard_line ))` |
//! | `CREATE TABLE` column list with `(` alone | `hard_line ( indent(hard_line col , hard_line col) hard_line )` |
//! | Trailing commas; a comment pins a comma | commas are tokens after items; `trailing_comment` then `, space item` |
//! | `CASE` | `group(CASE indent(line WHEN … line ELSE …) line END)` |
//! | Lambda breaks after `->` | `group(x -> indent(line body))` |
//! | A binary expression breaks before its operator | `group(a indent(line / b))` |
//! | `WITH TOTALS` on the keys' line, else its own | `if_break(keys, Gap::Line, Gap::Space)` |
//! | `CREATE MATERIALIZED VIEW … TO … AS`, `SELECT` at column 0 | `group(header indent(line TO t) space AS) hard_line` |
//! | `;` at the end of the last line, alone after a line comment | `;` after the last token (the line comment breaks the line) |
//! | Blank lines between statements kept 0–2, one around a multi-line statement | `hard_line blank_lines(n) blank_line_if_broken(previous) blank_line_if_broken(next)`, each statement a group |
//! | At most one blank line inside a statement, none after `(` or before `)` | `blank_lines(1)` where the input has one; the printer drops the rest |
//! | Trailing comment two spaces after the code | `trailing_comment` |
//! | Own-line comment at the indent of the next node | `own_line_comment` in the next node's scope |
//! | A line comment breaks its group; `/* */` in a line is an atom | automatic; `token` |
//! | A block of `--` lines keeps its relative indent | `comment_block` |
//! | Block tags on their own line, closers aligned; at column 0 between statements | `block_tag`, `block_tag_aligned`; `at_root` |
//! | `{{ }}` adjacency as written; whitespace next to a `-` marker fixed | automatic; `frozen`, `freeze` |
//! | Verbatim regions re-indented as a block; `::` operand and `INSERT` data untouched | `verbatim`; `verbatim_exact` |
//!
//! # Invariants for layout rules
//!
//! - Add every token other than whitespace once, in source order, and call
//!   [`Doc::check_tokens`] on every format: it names a lost or repeated
//!   comment where the verifier only sees a difference.
//! - Classify a comment by [`Builder::newlines_before`]: none means
//!   trailing, otherwise own-line, placed before the next item's tokens and
//!   after its line break, inside its scope.
//! - Put a line break inside the scope whose indentation it should take:
//!   `indent(line item)`, not `line indent(item)`.
//! - Give every statement its own group (reserved before its separator),
//!   so that `blank_line_if_broken` can see whether it spans lines.
//! - Ask for blank lines only where the style keeps them, with the input's
//!   count ([`Builder::newlines_before`] less one), at most two between
//!   statements and one inside them.
//! - Record every re-cased token for the verifier from [`Doc::recased`].
//! - In a template, freeze the whitespace the safe cleanup would keep (next
//!   to a `-` marker where a move across a line break changes the render);
//!   whitespace with a character the renderer does not strip is frozen
//!   automatically.

mod build;
mod check;
mod dump;
mod ir;
mod print;
mod touch;

pub use build::Builder;
pub use build::Open;
pub use check::TokenCheckError;
pub use print::PrintOptions;

use crate::lexer::Mode;
use crate::lexer::Token;
use crate::lexer::TokenKind;
use ir::GroupInfo;
use ir::Node;
use ir::VerbatimInfo;

/// A document: the layout of a text's tokens, ready to [`print`](Self::print).
///
/// Build one with a [`Builder`]. A document borrows the source text and its
/// tokens, and holds no copy of any token's text.
#[derive(Clone)]
pub struct Doc<'s> {
    source: &'s str,
    tokens: &'s [Token],
    mode: Mode,
    nodes: Vec<Node>,
    groups: Vec<GroupInfo>,
    verbatims: Vec<VerbatimInfo>,
    /// The texts of tokens printed other than as written.
    statics: Vec<&'static str>,
    /// For each block tag, the tag it aligns with, or [`ir::NONE`].
    tags: Vec<u32>,
    /// A bit per token: frozen (see [`Builder::freeze`]).
    frozen: Vec<u64>,
    ending: Ending,
}

/// A group of a document, to refer to from [`Builder::if_break`] and
/// [`Builder::blank_line_if_broken`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GroupId(u32);

/// A Jinja block tag of a document, for aligning a closer with its opener
/// (see [`Builder::block_tag_aligned`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TagId(u32);

/// Whitespace that [`Builder::if_break`] prints.
///
/// There is no conditional text: every token is printed exactly once, so
/// only whitespace can depend on a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Gap {
    /// Nothing.
    Nothing,
    /// A space.
    Space,
    /// As [`Builder::soft_line`]: nothing, or a line break where the
    /// enclosing group breaks.
    SoftLine,
    /// As [`Builder::line`]: a space, or a line break where the enclosing
    /// group breaks.
    Line,
    /// As [`Builder::hard_line`]: a line break. Either branch being a hard
    /// line keeps every group around it from printing flat.
    HardLine,
}

/// What follows the last token of the output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Ending {
    /// One `\n`, unless the tokens at the end of the output would lex
    /// differently with it, as ClickHouse lexes the last bytes of a text
    /// specially (`1e` at the very end is a bare word); then nothing.
    #[default]
    Newline,
    /// The text that follows the last token in the source, byte for byte:
    /// for a text that ends in `INSERT` data, whose end is where the data
    /// ends.
    AsInput,
}

impl<'s> Doc<'s> {
    /// The text the document lays out.
    #[must_use]
    pub fn source(&self) -> &'s str {
        self.source
    }

    /// The tokens of [`source`](Self::source) the document refers to by
    /// index.
    #[must_use]
    pub fn tokens(&self) -> &'s [Token] {
        self.tokens
    }

    /// The tokens the document prints in a case other than the source's:
    /// their indices, in order. The verifier must be told about each (see
    /// [`Facts::recase`](crate::verify::Facts::recase)).
    pub fn recased(&self) -> impl Iterator<Item = usize> + '_ {
        self.nodes.iter().filter_map(|node| match node {
            Node::Atom(atom) if atom.place == ir::Place::Inline && atom.aux != ir::NONE => {
                Some(atom.token as usize)
            }
            _ => None,
        })
    }

    /// Prints the document.
    ///
    /// The output never has whitespace at the end of a line, except inside
    /// a token, inside [frozen](Builder::freeze) whitespace and in
    /// [exact](Builder::verbatim_exact) verbatim text; it starts without a
    /// blank line (a byte-order mark the source starts with stays) and ends
    /// as [`Ending`] says.
    #[must_use]
    pub fn print(&self, options: &PrintOptions) -> String {
        print::print(self, options)
    }

    /// Checks that the document prints every token of the source other than
    /// whitespace exactly once, in the source's order: every comment and
    /// every Jinja tag among them.
    ///
    /// This is cheap (a pass over the document) and meant to run on every
    /// format, before the output is verified: it names the token a layout
    /// rule lost or repeated, where the verifier only sees that the output
    /// differs.
    ///
    /// # Errors
    ///
    /// The first token missing, repeated or out of order.
    pub fn check_tokens(&self) -> Result<(), TokenCheckError> {
        check::check_tokens(self)
    }

    fn token(&self, index: u32) -> Token {
        self.tokens[index as usize]
    }

    fn kind(&self, index: u32) -> TokenKind {
        self.token(index).kind()
    }

    fn is_frozen(&self, index: usize) -> bool {
        self.frozen
            .get(index / 64)
            .is_some_and(|word| word & (1 << (index % 64)) != 0)
    }
}

#[cfg(test)]
mod tests;
