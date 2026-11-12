//! The tolerant parser and the lossless syntax tree.
//!
//! [`parse`] turns a text into a [`rowan`] tree that holds every token of the
//! text, in order, so the tree's text is the input byte for byte. Statements
//! the parser knows are parsed into nodes; whatever it does not know, or
//! cannot parse, is kept verbatim, at the smallest level that works:
//!
//! 1. a list item ([`VerbatimItem`](SyntaxKind::VerbatimItem)) up to the
//!    next `,` or the end of the list,
//! 2. a clause ([`VerbatimClause`](SyntaxKind::VerbatimClause)) up to the
//!    next clause keyword,
//! 3. the whole statement ([`VerbatimStatement`](SyntaxKind::VerbatimStatement)).
//!
//! A parse never fails: [`Parse::fallbacks`] says what was kept verbatim and
//! why. Some text is [opaque](SyntaxKind::Opaque) by design, such as a
//! window specification.
//!
//! # Shape
//!
//! The [`Root`](SyntaxKind::Root) holds one [`Statement`](SyntaxKind::Statement)
//! per statement of the [split](mod@crate::split), with the trivia between
//! statements. A `Statement` holds its body, a node whose
//! [role](SyntaxKind::role) is [`Role::Statement`] or a `VerbatimStatement`,
//! and then the `;` that ends it, if there is one; a statement that is only a
//! `;` has no body.
//!
//! # Trivia
//!
//! Whitespace, comments and Jinja block and comment tags (`{% %}`, `{# #}`)
//! are trivia: the parser steps over them, and the tree builder places them
//! by one rule. **A node never starts or ends with trivia.** Trivia lies
//! between siblings, in the deepest node that holds the significant tokens
//! on both sides of it; trivia before the first statement or after the last
//! is in the `Root`. So a comment after a list item's comma is a child of
//! the list, between the comma and the next item; a comment between two
//! clauses is a child of the query, between the clauses; a comment before a
//! statement's `;` is a child of the `Statement`. A node's first and last
//! tokens are always significant, and the number of newlines in the
//! whitespace before a comment says whether it trails a line or stands on
//! its own.
//!
//! Jinja expressions (`{{ }}`) and words with tags glued into them
//! (`events_{{ suffix }}`) are significant tokens: atoms wherever a name or
//! an expression can stand.
//!
//! # Keyword slots
//!
//! ClickHouse has no reserved words, so whether a bare word is a keyword
//! depends on where it stands. A bare word the parser consumed in a keyword
//! slot has the kind [`Keyword`](SyntaxKind::Keyword) (canonically upper
//! case), and a literal word (`true`, `false`, `inf`, `infinity`, `nan`) the
//! kind [`LiteralWord`](SyntaxKind::LiteralWord) (canonically lower case);
//! every other bare word stays a [`BareWord`](SyntaxKind::BareWord), whose
//! case is never changed. Verbatim regions hold only the lexer's token
//! kinds, so nothing in them is re-cased. [`Parse::facts`] turns this into
//! what the [verifier](crate::verify) needs, and [`Parse::recased`] applies
//! it.
//!
//! # Text that must stay as written
//!
//! ClickHouse reads a few constructs as source text, so their whitespace is
//! meaningful; they are [`LiteralAsText`](SyntaxKind::LiteralAsText) nodes,
//! and [`Parse::facts`] records each as a region to keep byte for byte.

mod build;
pub(crate) mod grammar;
mod kind;
mod parser;
mod stats;

#[cfg(test)]
pub(crate) mod test_support;

use std::fmt::Write as _;

pub use kind::ClickHouse;
pub use kind::FallbackLevel;
pub use kind::Role;
pub use kind::SyntaxKind;
pub use parser::Fallback;
use rowan::GreenNode;
use rowan::NodeOrToken;
use rowan::WalkEvent;
pub use stats::StatementOutcome;
pub use stats::StatementStats;

use crate::lexer::Mode;
use crate::split::Split;
use crate::split::split;
use crate::verify::Facts;

/// A node of the syntax tree.
pub type SyntaxNode = rowan::SyntaxNode<ClickHouse>;
/// A token of the syntax tree.
pub type SyntaxToken = rowan::SyntaxToken<ClickHouse>;
/// A node or a token of the syntax tree.
pub type SyntaxElement = rowan::SyntaxElement<ClickHouse>;

/// What to parse each statement as.
///
/// [`Entry::File`] is what [`parse`] does; the others exist for tests, the
/// fuzzer and the corpus tool's measurements.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Entry {
    /// Statements, as in a file.
    #[default]
    File,
    /// The expression probe: a statement `SELECT <list>` with no clause
    /// keyword after the list is parsed as `SELECT` and an expression list,
    /// into an [`ExpressionProbe`](SyntaxKind::ExpressionProbe); every other
    /// statement is verbatim. It measures the expression grammar on real
    /// queries apart from the rest of the `SELECT` grammar.
    SelectListProbe,
    /// Each statement as one expression, with an optional alias.
    Expression,
    /// Each statement as one data type.
    DataType,
}

/// The result of parsing a text.
#[derive(Clone, Debug)]
pub struct Parse {
    green: GreenNode,
    fallbacks: Vec<Fallback>,
    facts: Facts,
    steps: u64,
}

/// Parses `source`; see the [module documentation](self).
///
/// # Panics
///
/// If `source` is 4 GiB or longer, as [`split`] does.
#[must_use]
pub fn parse(source: &str, mode: Mode) -> Parse {
    parse_split(source, &split(source, mode), Entry::File)
}

/// [`parse`] with each statement parsed as `entry`.
///
/// # Panics
///
/// If `source` is 4 GiB or longer, as [`split`] does.
#[must_use]
pub fn parse_as(source: &str, mode: Mode, entry: Entry) -> Parse {
    parse_split(source, &split(source, mode), entry)
}

/// Parses a text that has been split already: `split` is `split(source,
/// mode)`.
#[must_use]
pub fn parse_split(source: &str, split: &Split, entry: Entry) -> Parse {
    parse_with(source, split, &mut |p| grammar::statement(p, entry))
}

/// Parses each statement of `split` with `parse_statement`, which must
/// consume the statement's tokens up to its `;`.
pub(crate) fn parse_with(
    source: &str,
    split: &Split,
    parse_statement: &mut dyn FnMut(&mut parser::Parser<'_>),
) -> Parse {
    let tokens = split.tokens();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a text under 4 GiB has fewer than 4 Gi tokens"
    )]
    let significant: Vec<u32> = (0..tokens.len())
        .filter(|&i| tokens[i].kind().is_significant())
        .map(|i| i as u32)
        .collect();
    let index_of = |token: usize| significant.partition_point(|&i| (i as usize) < token);
    let mut p = parser::Parser::new(source, tokens, &significant);
    for statement in split.statements() {
        let start = index_of(statement.tokens.start);
        let end = index_of(statement.terminator.unwrap_or(statement.tokens.end));
        let m = p.start();
        p.set_range(start..end);
        parse_statement(&mut p);
        if statement.terminator.is_some() {
            p.set_range(end..end + 1);
            p.bump();
        }
        m.complete(&mut p, SyntaxKind::Statement);
    }
    let (events, fallbacks, steps) = p.finish();
    let (green, facts) = build::build(source, tokens, events);
    Parse {
        green,
        fallbacks,
        facts,
        steps,
    }
}

impl Parse {
    /// The tree's root, a [`Root`](SyntaxKind::Root) node.
    #[must_use]
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// How much work the parse took: how many times the parser looked at a
    /// token. It is at most a fixed multiple of the number of tokens, plus a
    /// constant per statement; a statement that would need more is kept
    /// verbatim. For tests and statistics that bound the parser's work.
    #[doc(hidden)]
    #[must_use]
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// The regions kept verbatim because they did not parse, in the order
    /// they were found, with the reason for each.
    #[must_use]
    pub fn fallbacks(&self) -> &[Fallback] {
        &self.fallbacks
    }

    /// What the verifier needs to know: the keyword slots, which may change
    /// case, and the [`LiteralAsText`](SyntaxKind::LiteralAsText) regions,
    /// which must be kept byte for byte. Tokens are numbered as in the
    /// text's [`Split::tokens`], which the tree holds one for one.
    #[must_use]
    pub fn facts(&self) -> &Facts {
        &self.facts
    }

    /// The text with every keyword slot in its canonical case: a
    /// [`Keyword`](SyntaxKind::Keyword) upper case, a
    /// [`LiteralWord`](SyntaxKind::LiteralWord) lower case, and nothing else
    /// changed.
    ///
    /// This is the re-casing the formatter will do, on its own: verifying it
    /// against the input with [`facts`](Self::facts), and checking that
    /// `clickhouse-format` prints both the same, tests the keyword slots.
    #[must_use]
    pub fn recased(&self) -> String {
        let root = self.syntax();
        let mut out = String::with_capacity(usize::from(root.text_range().len()));
        for token in root
            .descendants_with_tokens()
            .filter_map(NodeOrToken::into_token)
        {
            match token.kind() {
                SyntaxKind::Keyword => out.push_str(&token.text().to_ascii_uppercase()),
                SyntaxKind::LiteralWord => out.push_str(&token.text().to_ascii_lowercase()),
                _ => out.push_str(token.text()),
            }
        }
        out
    }

    /// What became of each statement.
    #[must_use]
    pub fn statements(&self) -> Vec<StatementStats> {
        self.syntax()
            .children()
            .filter(|n| n.kind() == SyntaxKind::Statement)
            .map(|n| stats::statement_stats(&n))
            .collect()
    }
}

/// The tree under `node` as indented text, one node or token per line with
/// its kind and byte range, and each token's text.
///
/// ```
/// use chairlift::lexer::Mode;
/// use chairlift::syntax::{debug_tree, parse};
///
/// let tree = debug_tree(&parse("DROP TABLE t;", Mode::Sql).syntax());
/// assert!(tree.starts_with("Root@0..13\n  Statement@0..13\n    DropQuery@0..12\n"));
/// ```
#[must_use]
pub fn debug_tree(node: &SyntaxNode) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for event in node.preorder_with_tokens() {
        match event {
            WalkEvent::Enter(element) => {
                out.extend(std::iter::repeat_n("  ", depth));
                let range = element.text_range();
                let _ = write!(
                    out,
                    "{}@{}..{}",
                    element.kind().name(),
                    u32::from(range.start()),
                    u32::from(range.end())
                );
                if let NodeOrToken::Token(token) = &element {
                    let _ = write!(out, " {:?}", token.text());
                }
                out.push('\n');
                if element.as_node().is_some() {
                    depth += 1;
                }
            }
            WalkEvent::Leave(element) => {
                if element.as_node().is_some() {
                    depth -= 1;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
