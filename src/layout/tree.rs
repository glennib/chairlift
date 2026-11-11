//! The syntax tree as the layout walks it: nodes numbered in preorder, with
//! facts about each node computed in one pass over the tree.
//!
//! The layout reads rowan's green tree, which holds no positions or parent
//! links and costs nothing to walk; token indices come from counting tokens
//! in order, and a node's facts from its preorder number.

use std::ops::Range;

use rowan::GreenNodeData;
use rowan::Language;
use rowan::NodeOrToken;

use crate::lexer::Token;
use crate::lexer::TokenKind;
use crate::lexer::jinja;
use crate::lexer::jinja::WhitespaceControl;
use crate::syntax::ClickHouse;
use crate::syntax::SyntaxKind;

/// A node of the syntax tree with its preorder number, under which
/// [`Tree`] keeps its facts.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Node<'g> {
    pub(crate) green: &'g GreenNodeData,
    pub(crate) id: u32,
}

impl Node<'_> {
    pub(crate) fn kind(&self) -> SyntaxKind {
        ClickHouse::kind_from_raw(self.green.kind())
    }
}

/// A child of a node that is not trivia: a node, or a token with its index
/// in the text's tokens.
#[derive(Clone, Copy, Debug)]
pub(crate) enum El<'g> {
    Node(Node<'g>),
    Token { kind: SyntaxKind, index: usize },
}

impl<'g> El<'g> {
    pub(crate) fn kind(&self) -> SyntaxKind {
        match self {
            El::Node(node) => node.kind(),
            El::Token { kind, .. } => *kind,
        }
    }

    pub(crate) fn node(&self) -> Option<Node<'g>> {
        match self {
            El::Node(node) => Some(*node),
            El::Token { .. } => None,
        }
    }

    pub(crate) fn is_token(&self, kind: SyntaxKind) -> bool {
        matches!(self, El::Token { kind: k, .. } if *k == kind)
    }
}

/// A child that is not trivia, with the whitespace and trivia between it
/// and the child before it (or the start of its parent), as token indices.
#[derive(Clone, Debug)]
pub(crate) struct Part<'g> {
    pub(crate) el: El<'g>,
    pub(crate) trivia: Range<usize>,
}

/// What a node contains, as bits of [`Info::flags`].
pub(crate) mod flag {
    /// A Jinja block tag between children the layout does not expect one
    /// between.
    pub(crate) const TAG_PLACE: u16 = 1 << 0;
    /// A Jinja block or comment tag touching the token next to it, on a
    /// side without a `-` marker.
    pub(crate) const TAG_TOUCHING: u16 = 1 << 1;
    /// A token that the rendered template joins with others (see
    /// [`Rendered::inside`](crate::cleanup::Rendered)).
    pub(crate) const RENDER_JOINED: u16 = 1 << 2;
    /// A token other than a tag with a Jinja block or comment tag inside
    /// it, such as `-- {% if x %}`, whose branches the layout cannot see.
    pub(crate) const HIDDEN_TAG: u16 = 1 << 3;
    /// A token other than whitespace that spans lines.
    pub(crate) const MULTILINE: u16 = 1 << 4;
    /// A token in or after `INSERT` data whose extent the rendering of the
    /// template can change (see [`Rendered::data`](crate::cleanup::Rendered)).
    pub(crate) const TEMPLATE_DATA: u16 = 1 << 5;
}

/// Facts about one node.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Info {
    /// The index of its first token.
    pub(crate) first: u32,
    /// Its number of tokens, whitespace included.
    pub(crate) len: u32,
    /// Its number of nodes, itself included.
    pub(crate) size: u32,
    /// What it contains, from [`flag`].
    pub(crate) flags: u16,
}

/// The facts about every node of a tree, by preorder number.
pub(crate) struct Tree {
    info: Vec<Info>,
}

impl Tree {
    /// Walks the tree under `root`, whose tokens are `tokens` of `source`;
    /// `joined` marks the tokens the rendered template joins with others.
    pub(crate) fn new(
        root: &GreenNodeData,
        source: &str,
        tokens: &[Token],
        joined: &[bool],
        data: &[bool],
    ) -> Tree {
        let mut info: Vec<Info> = Vec::with_capacity(tokens.len());
        let mut token = 0u32;
        // The open nodes, innermost last: their number, kind and children
        // still to walk.
        let mut open = vec![(0u32, kind(root), root.children())];
        info.push(Info {
            first: 0,
            len: 0,
            size: 0,
            flags: 0,
        });
        while let Some((id, parent, children)) = open.last_mut() {
            let (id, parent) = (*id, *parent);
            match children.next() {
                Some(NodeOrToken::Node(child)) => {
                    let child_id = narrow(info.len());
                    info.push(Info {
                        first: token,
                        len: 0,
                        size: 0,
                        flags: 0,
                    });
                    open.push((child_id, kind(child), child.children()));
                }
                Some(NodeOrToken::Token(t)) => {
                    let i = token as usize;
                    token += 1;
                    let kind = ClickHouse::kind_from_raw(t.kind());
                    info[id as usize].flags |= token_flags(source, tokens, i, kind, parent, joined);
                    if data.get(i).copied().unwrap_or(false) {
                        info[id as usize].flags |= flag::TEMPLATE_DATA;
                    }
                }
                None => {
                    open.pop();
                    let size = narrow(info.len()) - id;
                    let node = &mut info[id as usize];
                    node.len = token - node.first;
                    node.size = size;
                    let flags = node.flags;
                    if let Some(&(parent, _, _)) = open.last() {
                        info[parent as usize].flags |= flags;
                    }
                }
            }
        }
        Tree { info }
    }

    pub(crate) fn info(&self, node: &Node<'_>) -> Info {
        self.info[node.id as usize]
    }

    /// The root node, numbered 0.
    pub(crate) fn root(green: &GreenNodeData) -> Node<'_> {
        Node { green, id: 0 }
    }

    /// The children of `node` that are not trivia, each with the trivia
    /// before it, and the trivia after the last one (only a
    /// [`Root`](SyntaxKind::Root) can end with trivia).
    pub(crate) fn parts<'g>(&self, node: &Node<'g>) -> (Vec<Part<'g>>, Range<usize>) {
        let info = self.info(node);
        let mut parts = Vec::with_capacity(node.green.children().len());
        let mut id = node.id + 1;
        let mut token = info.first as usize;
        let mut trivia_start = token;
        for child in node.green.children() {
            match child {
                NodeOrToken::Token(t) => {
                    let kind = ClickHouse::kind_from_raw(t.kind());
                    if !kind.is_trivia() {
                        parts.push(Part {
                            el: El::Token { kind, index: token },
                            trivia: trivia_start..token,
                        });
                        trivia_start = token + 1;
                    }
                    token += 1;
                }
                NodeOrToken::Node(green) => {
                    let child = self.info[id as usize];
                    parts.push(Part {
                        el: El::Node(Node { green, id }),
                        trivia: trivia_start..token,
                    });
                    token += child.len as usize;
                    trivia_start = token;
                    id += child.size;
                }
            }
        }
        (parts, trivia_start..token)
    }
}

fn kind(green: &GreenNodeData) -> SyntaxKind {
    ClickHouse::kind_from_raw(green.kind())
}

/// What token `i`, a child of a node of `parent` kind, adds to its parent's
/// flags.
fn token_flags(
    source: &str,
    tokens: &[Token],
    i: usize,
    kind: SyntaxKind,
    parent: SyntaxKind,
    joined: &[bool],
) -> u16 {
    let mut flags = 0;
    if kind != SyntaxKind::Whitespace && tokens[i].text(source).contains('\n') {
        flags |= flag::MULTILINE;
    }
    if joined.get(i).copied().unwrap_or(false) {
        flags |= flag::RENDER_JOINED;
    }
    match kind {
        SyntaxKind::Whitespace | SyntaxKind::JinjaExpr | SyntaxKind::JinjaRaw => {}
        SyntaxKind::JinjaComment => {
            if touching(source, tokens, i) {
                flags |= flag::TAG_TOUCHING;
            }
        }
        SyntaxKind::JinjaBlock => {
            if !tag_allowed_in(parent) {
                flags |= flag::TAG_PLACE;
            }
            if touching(source, tokens, i) {
                flags |= flag::TAG_TOUCHING;
            }
        }
        _ => {
            if has_hidden_tag(tokens[i].text(source)) {
                flags |= flag::HIDDEN_TAG;
            }
        }
    }
    flags
}

/// Whether a block tag may stand between two children of a node of `kind`:
/// between statements, clauses, list items, and `AND`/`OR` operands, where
/// the layout puts it on a line of its own at the indentation of what
/// follows.
pub(crate) fn tag_allowed_in(kind: SyntaxKind) -> bool {
    kind.is_list()
        || matches!(
            kind,
            SyntaxKind::Root
                | SyntaxKind::Statement
                | SyntaxKind::SelectQuery
                | SyntaxKind::UnionQuery
                | SyntaxKind::AndExpr
                | SyntaxKind::OrExpr
                | SyntaxKind::CreateTable
                | SyntaxKind::CreateMaterializedView
                | SyntaxKind::AsClause
                | SyntaxKind::InsertQuery
                | SyntaxKind::CreateView
                | SyntaxKind::CreateDictionary
                | SyntaxKind::DeleteQuery
                | SyntaxKind::UpdateQuery
                | SyntaxKind::GrantQuery
                | SyntaxKind::AccessEntityQuery
        )
}

/// Whether `text`, a token that is not a tag, holds a block or comment tag
/// that the renderer acts on.
fn has_hidden_tag(text: &str) -> bool {
    (text.contains("{%") || text.contains("{#"))
        && jinja::tags(text)
            .any(|t| matches!(t.kind, jinja::TagKind::Block | jinja::TagKind::Comment))
}

/// Whether tag token `i` touches the token before or after it on a side
/// without a `-` marker, which strips the whitespace there anyway.
fn touching(source: &str, tokens: &[Token], i: usize) -> bool {
    let (before, after) = touches(source, tokens, i);
    before || after
}

/// Whether tag token `i` touches the token before it and the token after
/// it, on a side without a `-` marker.
pub(crate) fn touches(source: &str, tokens: &[Token], i: usize) -> (bool, bool) {
    let text = tokens[i].text(source);
    let Some(tag) = jinja::tags(text).next() else {
        return (false, false);
    };
    let (before, after) = tag.whitespace_control(text);
    let solid = |j: Option<usize>| {
        j.and_then(|j| tokens.get(j))
            .is_some_and(|t| t.kind() != TokenKind::Whitespace)
    };
    (
        before != WhitespaceControl::Strip && solid(i.checked_sub(1)),
        after != WhitespaceControl::Strip && solid(Some(i + 1)),
    )
}

pub(crate) fn narrow(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
