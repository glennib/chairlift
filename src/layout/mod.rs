//! The layout: from a syntax tree to a [document](crate::doc).
//!
//! [`format`](crate::format) splits and parses a text, lays out the tree
//! here, prints the document and verifies the result. The layout walks the
//! tree once, adding every token to a [`Builder`](crate::doc::Builder) in
//! order, with the spaces, line breaks, groups and indentation the style
//! (`docs/MILESTONES.md`, "Style") asks for between them.
//!
//! # Statements: laid out or kept as written
//!
//! Each statement is either laid out by the rule for its kind or kept as
//! written, which prints it as the safe cleanup does (see
//! [`Builder::as_written`](crate::doc::Builder::as_written)): its lines as
//! they are, without trailing whitespace and with at most one blank line
//! in a row. The layout never guesses: a statement is kept when the parser
//! did not parse it, when there is no rule for its kind yet, or when it
//! holds a Jinja shape the rules do not cover, and [`Statement::decision`]
//! says which. A kept statement still starts on a line of its own, with
//! the blank lines between statements that the style keeps.
//!
//! # How the parts fit
//!
//! | Module | What it lays out |
//! | --- | --- |
//! | `file` | the statements, what lies between them, which are kept as written |
//! | `writer` | tokens and keyword case, comments, block tags and blank lines between children, the generic one-line rule and its spacing, closers that start a line, text kept on one line |
//! | `shapes` | lists, clauses of a keyword and a list, hugging brackets |
//! | `expr` | binary operators, `AND`/`OR` runs, `BETWEEN`, `CASE`, lambdas, `?:`, casts and data types |
//! | `query` | `SELECT` queries, clauses, joins, set operations, `WITH` |
//! | `create` | `CREATE TABLE`, views and dictionaries: header line, element list, column definitions, storage clauses, `AS` |
//! | `insert` | `INSERT`: header line, data, the query after it |
//! | `simple` | `DROP`, `SET`, `RENAME`, `OPTIMIZE` and other statements of a line or two |
//! | `alter` | `ALTER` and its commands, the lightweight `DELETE` and `UPDATE` |
//! | `create_other` | `CREATE DATABASE`, the clauses of `CREATE DICTIONARY` |
//! | `access` | `GRANT`, `REVOKE`, `CREATE` and `ALTER` of users, roles, quotas, row policies and settings profiles |
//! | `tree` | preorder node numbers and the facts about each node, from one pass |
//! | `comment_end` | the template line comments the rendering can make run on, which leave the text as it is |
//! | `joins` | the template words a render can join to a token beyond a block tag, which keep their case |
//! | `inner` | block tags inside a string or comment whose blocks stay inside it |
//!
//! `node` dispatches a node to its rule by kind. A kind without a rule
//! of its own is laid out by the generic rule
//! (`Writer::generic`): its children on one
//! line, spaced as ClickHouse prints them (no space around `.` and `::`,
//! inside brackets, before `,`, or before a call's arguments), and each
//! child by its own rule. Verbatim and opaque nodes are printed as written,
//! re-indented as a block; a [`LiteralAsText`](SyntaxKind::LiteralAsText)
//! byte for byte.
//!
//! # Trivia
//!
//! Comments and Jinja block tags are children of the deepest node that
//! holds the tokens on both sides of them (see
//! [`syntax`](crate::syntax#trivia)). Every rule that puts something
//! between two children calls `Writer::gap` with the
//! trivia between them and the separator it wants; `gap` keeps a comment
//! that trails its line on it, puts the separator, then puts the comments
//! and tags on lines of their own in the scope that is open, which is the
//! scope of what follows. So a rule only has to open the scope of an item
//! before the separator in front of it: `indent(line item)`. `gap` also
//! says whether the trivia ended the line, which is how a comment pins a
//! comma to the start of the next line.
//!
//! # Adding a rule
//!
//! - **A statement kind**: give `file::rule_for` the kind of the
//!   statement's body and the function that lays it out. Until a kind has a
//!   rule, its statements are kept as written with [`Kept::NoRule`]. A
//!   statement's rule is called inside the statement's group; it lays out
//!   the body at the indentation it is called in, and the file layout adds
//!   the `;`.
//! - **A clause**: add its kind to `node`. A clause of keywords and a
//!   list is `query::list_clause`; one of keywords and an expression is
//!   the generic rule. Clauses go one per line because the rule for the
//!   node that holds them (`query::clauses` for a query,
//!   `create::statement` for a `CREATE`) puts a hard line between its
//!   children.
//! - **An expression or another node**: add its kind to `node`, with a
//!   function that takes the node's children from
//!   `Writer::parts` and calls `Writer::gap` between them. Use a group for
//!   what should stay on one line if it fits, and put a line break inside
//!   the scope that should indent it.
//! - **A query inside a statement** (`AS SELECT`, a materialized view's
//!   body, `INSERT … SELECT`): end the header's line with a hard line and
//!   call `query::query` at column 0, as `create::as_clause` and
//!   `insert::statement` do. A query in brackets is `shapes::hugged`.
//!
//! Jinja block tags are allowed only between the children of the node
//! kinds in `tree::tag_allowed_in`; a statement with a tag anywhere else
//! is kept as written. Extend it with the node kinds whose rule puts a line
//! break between children, or passes the trivia before a body to
//! `Writer::body_scope`.
//!
//! # Templates
//!
//! What the layout does with Jinja, by `docs/MILESTONES.md` ("Style",
//! Jinja):
//!
//! - A block tag goes on a line of its own at the indentation of what
//!   follows it, `{% else %}`, `{% elif %}` and closers at their opener's;
//!   between statements, at column 0. A `{% for %}` that emits list items
//!   is at the items' indentation, and the items are not indented further.
//! - A statement's header, the `{{ … }}` lines before it (dbt's
//!   `{{ config(…) }}`), goes at column 0 with the blank line after it kept;
//!   one that follows the statement before on its line (a test hint after
//!   `;`) stays there.
//! - Whitespace next to a `{{ }}` stays present or absent; a tag touching a
//!   token keeps its statement as written, unless the token is a bracket,
//!   `,` or `;`, which a render never joins to anything.
//! - A statement is kept as written for a tag where `tag_allowed_in` does
//!   not allow it ([`Kept::TagPlace`]), a tag touching a word or literal
//!   ([`Kept::TagTouching`]), text the render with every branch taken joins
//!   into one token ([`Kept::RenderJoined`]), `INSERT … FORMAT` data whose
//!   extent a branch can change and what follows it up to a blank line every
//!   render keeps ([`Kept::TemplateData`]); a whole text for a block or
//!   comment tag inside another token whose block reaches outside it
//!   ([`Kept::HiddenTag`], see `inner`).
//! - A word that a render can join to a token beyond a block tag keeps its
//!   case (see `joins`), and a text in which a render can make a line
//!   comment run on is refused (see `comment_end`).
//!
//! The verifier checks the render with every branch taken; `chairlift::template`
//! (the `template` feature) renders every combination of branches, and the
//! corpus tool and `tests/template_properties.rs` compare those renders.

mod access;
mod alter;
mod comment_end;
mod create;
mod create_other;
mod expr;
mod file;
mod inner;
mod insert;
mod joins;
mod query;
mod shapes;
mod simple;
mod tree;
mod writer;

use std::ops::Range;

use crate::cleanup::rendered_facts;
use crate::doc::Doc;
use crate::doc::Ending;
use crate::lexer::Mode;
use crate::split::DataFormat;
use crate::split::Split;
use crate::syntax::Parse;
use crate::syntax::SyntaxKind;
pub(crate) use comment_end::comment_ended_by_rendering;
use tree::Node;
use tree::Tree;
use tree::flag;
pub(crate) use writer::TagRole;
use writer::Writer;
pub(crate) use writer::tag_role;

/// What the layout did with one statement of a text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    /// Its tokens: indices into the text's
    /// [`Split::tokens`](crate::split::Split::tokens), from its first
    /// significant token through its `;`.
    pub tokens: Range<usize>,
    /// Whether it was laid out, or kept as written and why.
    pub decision: Decision,
}

/// Whether a statement was laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Decision {
    /// Laid out by the rules for its kind.
    Formatted,
    /// Kept as written, as the safe cleanup prints it.
    Kept(Kept),
}

/// Why a statement was kept as written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kept {
    /// The parser kept it verbatim: a kind it does not know, or one that
    /// did not parse.
    NotParsed,
    /// The layout has no rule for its kind yet.
    NoRule,
    /// A Jinja block tag stands where the layout cannot put it on a line
    /// of its own, such as inside an expression.
    TagPlace,
    /// A Jinja block or comment tag touches the token next to it, other than
    /// a bracket, `,` or `;`, so that moving it to a line of its own could
    /// change what the template renders.
    TagTouching,
    /// Rendering the template joins its tokens into others: a string or
    /// heredoc around a tag.
    RenderJoined,
    /// It is in `INSERT … FORMAT` data in a template whose rendering can
    /// change where the data starts or ends, or before the blank line that
    /// ends the data in every render (after it, where there is none).
    TemplateData,
    /// The text has a Jinja block or comment tag inside a comment, string
    /// or word, whose branches the layout cannot see: one whose block
    /// reaches outside that token, or whose render can change where the
    /// token ends.
    HiddenTag,
}

impl Decision {
    /// A short name, for tables and CSV: `formatted`, or the reason it was
    /// kept.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Decision::Formatted => "formatted",
            Decision::Kept(kept) => kept.name(),
        }
    }
}

impl Kept {
    /// A short name, for tables and CSV.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Kept::NotParsed => "not_parsed",
            Kept::NoRule => "no_rule",
            Kept::TagPlace => "tag_place",
            Kept::TagTouching => "tag_touching",
            Kept::RenderJoined => "render_joined",
            Kept::TemplateData => "template_data",
            Kept::HiddenTag => "hidden_tag",
        }
    }
}

/// A text laid out: the document and what became of each statement.
pub(crate) struct Layout<'s> {
    pub(crate) doc: Doc<'s>,
    pub(crate) statements: Vec<Statement>,
}

/// The document [`format`](crate::format) prints for `source`, which
/// `split` and `parse` are of, lexed in `mode`: the layout on its own, for
/// measuring it.
#[must_use]
pub fn document<'s>(source: &'s str, split: &'s Split, parse: &Parse, mode: Mode) -> Doc<'s> {
    layout(source, split, parse, mode).doc
}

/// Lays out `source`, which `split` and `parse` are of.
pub(crate) fn layout<'s>(
    source: &'s str,
    split: &'s Split,
    parse: &Parse,
    mode: Mode,
) -> Layout<'s> {
    let tokens = split.tokens();
    let rendered = rendered_facts(source, split, mode);
    let syntax = parse.syntax();
    let free = if mode == Mode::Jinja {
        joins::free_punctuation(source, tokens)
    } else {
        Vec::new()
    };
    let tree = Tree::new(
        syntax.green(),
        source,
        tokens,
        &rendered.inside,
        &rendered.data,
        mode == Mode::Jinja,
        &free,
    );
    let root = Tree::root(syntax.green());
    let mut w = Writer::new(source, tokens, mode, tree);
    let (parts, _) = w.parts_and_tail(&root);
    let keep_all = w.has(&root, flag::HIDDEN_TAG).then_some(Kept::HiddenTag);
    let plan = file::plan(&w, &parts, keep_all);
    freeze(&mut w, &plan, &rendered, parse, keep_all.is_some(), &free);
    file::root(&mut w, &root, &plan);
    let ending = ending(source, split, rendered.open_data);
    w.b.set_ending(ending);
    let statements = file::report(&w, &plan);
    Layout {
        doc: w.b.finish(),
        statements,
    }
}

/// Freezes the whitespace that must stay as it is: in statements kept as
/// written, and everywhere in a text all of whose statements are kept for
/// what it holds (`keep_all`), what the safe cleanup keeps; everywhere,
/// what the rendered template joins into other tokens, and a tag and the
/// token it touches.
fn freeze(
    w: &mut Writer<'_>,
    plan: &[file::Planned],
    rendered: &crate::cleanup::Rendered,
    parse: &Parse,
    keep_all: bool,
    free: &[bool],
) {
    let mut frozen: Vec<bool> = if keep_all {
        rendered.kept.clone()
    } else {
        rendered
            .inside
            .iter()
            .zip(&rendered.data)
            .map(|(a, b)| *a || *b)
            .collect()
    };
    // The token ranges of the statements kept as written, in order.
    let kept: Vec<(usize, usize)> = plan
        .iter()
        .filter(|p| p.decision != Decision::Formatted)
        .map(|p| w.range(&p.node))
        .collect();
    for &(first, last) in &kept {
        if let Some(kept) = rendered.kept.get(first..=last) {
            for (f, &k) in frozen[first..=last].iter_mut().zip(kept) {
                *f |= k;
            }
        }
    }
    for &(a, b) in parse.facts().verbatim_regions() {
        let i = kept.partition_point(|&(_, last)| last < a);
        if kept
            .get(i)
            .is_some_and(|&(first, last)| first <= a && b <= last)
        {
            if frozen.len() <= b {
                frozen.resize(b + 1, false);
            }
            frozen[a..=b].fill(true);
        }
    }
    let mut indices: Vec<usize> = frozen
        .iter()
        .enumerate()
        .filter_map(|(i, &f)| f.then_some(i))
        .collect();
    indices.extend(w.touching_tags(free));
    indices.sort_unstable();
    indices.dedup();
    for i in indices {
        w.freeze(i);
    }
}

/// What follows the last token: what the safe cleanup leaves there.
fn ending(source: &str, split: &Split, rendered_open_data: bool) -> Ending {
    let tokens = split.tokens();
    let Some(last_solid) = tokens
        .iter()
        .rposition(|t| t.kind() != crate::lexer::TokenKind::Whitespace)
    else {
        return Ending::Newline;
    };
    let open_data = split
        .statements()
        .last()
        .and_then(|s| s.data)
        .is_some_and(|data| data.token == last_solid && data.format == DataFormat::Other);
    if rendered_open_data {
        Ending::AsInput
    } else if open_data {
        if tokens[last_solid].end() < source.len() {
            Ending::BlankLine
        } else {
            Ending::AsInput
        }
    } else {
        Ending::Newline
    }
}

/// Lays out `node` by the rule for its kind.
pub(crate) fn node(w: &mut Writer<'_>, node: &Node) {
    use SyntaxKind as S;
    let kind = node.kind();
    if kind == S::LiteralAsText {
        w.exact(node);
        return;
    }
    if kind.is_verbatim() {
        w.verbatim(node);
        return;
    }
    match kind {
        S::SelectQuery | S::UnionQuery => query::clauses(w, node),
        S::Subquery => shapes::hugged(w, node),
        S::SelectClause => query::select_clause(w, node),
        S::WithClause => query::with_clause(w, node),
        S::TableElementList | S::DictionaryAttributeList => create::element_list(w, node),
        S::ColumnDef => create::column(w, node),
        S::IndexDef => create::index(w, node),
        S::ProjectionDef => create::projection(w, node),
        S::ColumnCodec | S::ColumnStatistics => create::codec(w, node),
        S::OrderByClause if create::is_table_order_by(w, node) => query::keyword_clause(w, node),
        S::OrderByTuple => create::order_by_tuple(w, node),
        S::RefreshClause => create::refresh(w, node),
        S::TtlElement => create::ttl_rule(w, node),
        S::SqlSecurityClause => create::sql_security(w, node),
        S::AsClause => create::as_clause(w, node),
        S::CreateHeader => create::header(w, node),
        S::AlterCommand => alter::command(w, node),
        S::DictionaryClause => create_other::clause(w, node),
        S::KeyValueFunction => create_other::key_value_function(w, node),
        S::ViewTargetClause => create::view_target(w, node),
        S::FromClause
        | S::ArrayJoinClause
        | S::OrderByClause
        | S::SettingsClause
        | S::WindowClause
        | S::TtlClause
        | S::LimitByClause
        | S::UpdateSetClause => query::list_clause(w, node),
        S::GroupByClause => query::group_by(w, node),
        S::JoinClause => query::join(w, node),
        S::ParenJoin => query::paren_join(w, node),
        S::BinaryExpr => expr::binary(w, node),
        S::AndExpr | S::OrExpr => expr::hanging(w, node),
        S::CaseExpr => expr::case(w, node),
        S::LambdaExpr => expr::lambda(w, node),
        S::TernaryExpr => expr::ternary(w, node),
        S::CastExpr => expr::cast(w, node),
        S::BetweenExpr => expr::between(w, node),
        S::DataType => expr::data_type(w, node),
        S::UserName | S::RoleKeyword | S::GrantTarget => access::glued(w, node),
        S::Privilege | S::CurrentGrants => access::privilege(w, node),
        S::QuotaIntervalList => access::intervals(w, node),
        S::AccessClause => access::clause(w, node),
        S::AuthMethod => create::breaking(w, node, |w, el| {
            shapes::keyword_is(w, el, &["SALT", "VALID"])
        }),
        _ if kind.is_list() => {
            let (_, last) = w.range(node);
            if matches!(
                w.token_kind(last),
                crate::lexer::TokenKind::ClosingRoundBracket
                    | crate::lexer::TokenKind::ClosingSquareBracket
                    | crate::lexer::TokenKind::ClosingCurlyBrace
            ) {
                shapes::bracketed(w, node);
            } else {
                shapes::plain_list(w, node);
            }
        }
        _ if kind.is_clause() => query::keyword_clause(w, node),
        _ => w.generic(node),
    }
}

#[cfg(test)]
mod tests;
