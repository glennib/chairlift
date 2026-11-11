//! The layout's writer: a document [`Builder`] with the tree it lays out,
//! and the parts every layout rule shares: tokens with their keyword case,
//! comments, Jinja block tags and blank lines between children, and text
//! kept as written.

use std::ops::Range;

use super::tree::El;
use super::tree::Info;
use super::tree::Node;
use super::tree::Part;
use super::tree::Tree;
use crate::doc::Builder;
use crate::doc::Gap;
use crate::doc::GroupId;
use crate::doc::Open;
use crate::doc::TagId;
use crate::lexer::Mode;
use crate::lexer::Token;
use crate::lexer::TokenKind;
use crate::lexer::jinja;
use crate::syntax::SyntaxKind;

/// What a layout rule puts between two children, before the trivia between
/// them is placed (see [`Writer::gap`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sep {
    /// Nothing: `f(`, `a,`, `t.c`.
    Nothing,
    /// A space.
    Space,
    /// A space, or a line break where the enclosing group breaks.
    Line,
    /// Nothing, or a line break where the enclosing group breaks.
    Soft,
    /// A line break.
    Hard,
}

/// Lays out a tree into a document.
pub(crate) struct Writer<'s> {
    pub(crate) b: Builder<'s>,
    source: &'s str,
    tokens: &'s [Token],
    tree: Tree,
    /// The Jinja blocks open at this point, innermost last: the tag that
    /// opened each, or `None` for one opened inside text printed as written.
    blocks: Vec<Option<TagId>>,
    /// A token has been added.
    started: bool,
    /// The tokens frozen, for [`freeze`](Self::freeze).
    frozen: Vec<bool>,
    /// Between statements, where blank lines are kept next to brackets too.
    pub(crate) between_statements: bool,
    /// The places a line can break, counted as they are added.
    breaks: u32,
    /// The last closer added that starts a line of its own when its list or
    /// `CASE` breaks.
    closer: Option<Closer>,
    /// How many [`on_one_line`](Self::on_one_line) scopes are open.
    one_line: u32,
    /// How many data types the node being laid out is inside.
    pub(crate) type_depth: u32,
    /// The words that keep their case, even in a keyword slot: those a render
    /// can join to a token beyond a block tag (see `super::joins`).
    keep_case: Vec<bool>,
}

/// A closer that starts a line of its own when its list or `CASE` breaks.
#[derive(Clone, Copy)]
struct Closer {
    token: usize,
    line: CloserLine,
    /// [`Writer::breaks`] when it was added.
    breaks: u32,
}

/// When a closer starts a line of its own.
#[derive(Clone, Copy)]
pub(crate) enum CloserLine {
    /// Always, as a subquery's `)` does.
    Always,
    /// When the group breaks, as the `)` of a list and `END` do.
    IfBroken(GroupId),
}

impl<'s> Writer<'s> {
    pub(crate) fn new(source: &'s str, tokens: &'s [Token], mode: Mode, tree: Tree) -> Writer<'s> {
        let keep_case = if mode == Mode::Jinja {
            super::joins::joinable(source, tokens)
        } else {
            Vec::new()
        };
        Writer {
            b: Builder::new(source, tokens, mode),
            source,
            tokens,
            tree,
            blocks: Vec::new(),
            started: false,
            frozen: Vec::new(),
            between_statements: false,
            breaks: 0,
            closer: None,
            one_line: 0,
            type_depth: 0,
            keep_case,
        }
    }

    /// Lays out what `f` adds on one line: where a line could break, a space
    /// or nothing; only a comment still ends a line.
    pub(crate) fn on_one_line(&mut self, f: impl FnOnce(&mut Self)) {
        self.one_line += 1;
        f(self);
        self.one_line -= 1;
    }

    // Closers.

    /// Records that token `index`, just added, is a closer that starts a line
    /// of its own as `line` says, unless what is before it can keep it off
    /// the start of a line: frozen whitespace, or a Jinja tag it touches.
    pub(crate) fn closed(&mut self, index: usize, line: CloserLine) {
        let held = index.checked_sub(1).is_some_and(|i| {
            self.frozen.get(i).copied().unwrap_or(false)
                || (self.tokens[i].kind() != TokenKind::Whitespace
                    && jinja::tags(self.text(i)).next().is_some())
        });
        self.closer = (!held).then_some(Closer {
            token: index,
            line,
            breaks: self.breaks,
        });
    }

    /// When the line being written starts with a closer at or after token
    /// `first`, with no place to break after it.
    pub(crate) fn closer_line(&self, first: usize) -> Option<CloserLine> {
        self.closer
            .filter(|c| c.token >= first && c.breaks == self.breaks)
            .map(|c| c.line)
    }

    /// Adds the place a line can break before what follows a closer that
    /// starts its line as `closer` says, and opens a group, which the caller
    /// closes after what should stay on the closer's line: it stays there if
    /// it fits with what follows it up to the next place a line can break.
    /// Where the closer does not start its line, the line is the enclosing
    /// group's, as anywhere else.
    pub(crate) fn open_after_closer(&mut self, closer: CloserLine) -> Open {
        self.breaks += 1;
        match closer {
            CloserLine::Always => {
                let open = self.b.open_group();
                self.b.line();
                open
            }
            CloserLine::IfBroken(group) => {
                self.b.if_break(group, Gap::Nothing, Gap::Line);
                let open = self.b.open_group();
                self.b.if_break(group, Gap::Line, Gap::Nothing);
                open
            }
        }
    }

    /// Whether `trivia` holds whitespace only.
    pub(crate) fn only_whitespace(&self, trivia: &Range<usize>) -> bool {
        trivia
            .clone()
            .all(|i| self.tokens[i].kind() == TokenKind::Whitespace)
    }

    /// Adds what comes between the last item of a list or query and its
    /// closer, and closes `items`, the scope the items are laid out in: the
    /// comments and tags of `trivia` on lines of their own in that scope, at
    /// the items' indentation, then `sep` before the closer, outside it.
    pub(crate) fn gap_before_closer(&mut self, trivia: Range<usize>, items: Open, sep: Sep) {
        if self.only_whitespace(&trivia) {
            self.b.close(items);
            self.gap(trivia, sep, 0);
        } else {
            self.gap(trivia, sep, 0);
            self.b.close(items);
            self.sep(sep);
        }
    }

    // The tree.

    pub(crate) fn parts<'g>(&self, node: &Node<'g>) -> Vec<Part<'g>> {
        self.tree.parts(node).0
    }

    pub(crate) fn parts_and_tail<'g>(&self, node: &Node<'g>) -> (Vec<Part<'g>>, Range<usize>) {
        self.tree.parts(node)
    }

    pub(crate) fn info(&self, node: &Node) -> Info {
        self.tree.info(node)
    }

    pub(crate) fn has(&self, node: &Node, flag: u16) -> bool {
        self.info(node).flags & flag != 0
    }

    /// The indices of a node's first and last tokens, which are never
    /// trivia.
    pub(crate) fn range(&self, node: &Node) -> (usize, usize) {
        let info = self.info(node);
        let first = info.first as usize;
        (first, first + info.len as usize - 1)
    }

    /// The lexer's kind of the first token of `el`.
    pub(crate) fn first_kind(&self, el: &El) -> TokenKind {
        let i = match el {
            El::Node(node) => self.range(node).0,
            El::Token { index, .. } => *index,
        };
        self.tokens[i].kind()
    }

    /// The lexer's kind of the last token of `el`.
    pub(crate) fn last_kind(&self, el: &El) -> TokenKind {
        let i = match el {
            El::Node(node) => self.range(node).1,
            El::Token { index, .. } => *index,
        };
        self.tokens[i].kind()
    }

    /// The text of token `index`.
    pub(crate) fn text(&self, index: usize) -> &'s str {
        self.tokens[index].text(self.source)
    }

    /// The lexer's kind of token `index`.
    pub(crate) fn token_kind(&self, index: usize) -> TokenKind {
        self.tokens[index].kind()
    }

    /// Freezes token `index` (see [`Builder::freeze`]); call it before
    /// adding the tokens it concerns.
    pub(crate) fn freeze(&mut self, index: usize) {
        if self.frozen.len() <= index {
            self.frozen.resize(index + 1, false);
        }
        self.frozen[index] = true;
        self.b.freeze(index);
    }

    /// Prints the whitespace before token `index` as in the input, here, if
    /// it is frozen: so that a group opened after this does not start with
    /// the line break, which would count as one inside it.
    pub(crate) fn frozen_before(&mut self, index: usize) {
        let ws = index.wrapping_sub(1);
        if self.frozen.get(ws).copied().unwrap_or(false)
            && self.tokens[ws].kind() == TokenKind::Whitespace
        {
            self.b.frozen();
        }
    }

    /// The number of line breaks in the whitespace before token `index`.
    pub(crate) fn newlines_before(&self, index: usize) -> usize {
        self.b.newlines_before(index)
    }

    // Tokens.

    /// Adds token `index`, of syntax kind `kind`: a keyword in a keyword
    /// slot upper case, a literal word in its canonical case, anything else,
    /// and a word a render can join to another, as written.
    pub(crate) fn token(&mut self, kind: SyntaxKind, index: usize) {
        self.started = true;
        if self.keep_case.get(index).copied().unwrap_or(false) {
            self.b.token(index);
            return;
        }
        match kind {
            SyntaxKind::Keyword if self.tokens[index].keyword().is_some() => self.b.keyword(index),
            SyntaxKind::LiteralWord => self.b.literal_word(index),
            _ => self.b.token(index),
        }
    }

    /// Adds `el`: a token, or a node laid out by its rule.
    pub(crate) fn el(&mut self, el: &El) {
        match el {
            El::Token { kind, index } => self.token(*kind, *index),
            El::Node(node) => super::node(self, node),
        }
    }

    /// Adds `node`'s tokens as written, re-indented as a block.
    pub(crate) fn verbatim(&mut self, node: &Node) {
        let (first, last) = self.range(node);
        self.verbatim_range(first, last);
    }

    pub(crate) fn verbatim_range(&mut self, first: usize, last: usize) {
        self.started = true;
        self.breaks += 1;
        self.track_tags(first, last);
        self.b.verbatim(first, last);
    }

    /// Adds `node`'s tokens byte for byte.
    pub(crate) fn exact(&mut self, node: &Node) {
        let (first, last) = self.range(node);
        self.started = true;
        self.b.verbatim_exact(first, last);
    }

    /// Adds tokens `first` to `last` on the lines they are on in the input.
    pub(crate) fn as_written(&mut self, first: usize, last: usize) {
        self.started = true;
        self.breaks += 1;
        self.track_tags(first, last);
        self.b.as_written(first, last);
    }

    // Separators and trivia.

    /// Adds `sep`, counting it if a line can break there.
    pub(crate) fn sep(&mut self, sep: Sep) {
        let sep = match sep {
            Sep::Line if self.one_line > 0 => Sep::Space,
            Sep::Soft if self.one_line > 0 => Sep::Nothing,
            sep => sep,
        };
        self.breaks += u32::from(matches!(sep, Sep::Line | Sep::Soft | Sep::Hard));
        match sep {
            Sep::Nothing => {}
            Sep::Space => self.b.space(),
            Sep::Line => self.b.line(),
            Sep::Soft => self.b.soft_line(),
            Sep::Hard => self.b.hard_line(),
        }
    }

    /// Adds what goes between two children: the trivia in `trivia` (token
    /// indices, whitespace included) and the separator `sep` the layout
    /// wants there, keeping up to `blank` of the input's blank lines.
    ///
    /// Comments that trail the line of the code before them stay on it, two
    /// spaces after the code (a `/* */` one is an atom, a space after the
    /// code); then comes `sep`; then the comments and block tags on lines of
    /// their own, at the indentation of the scope this is called in, which
    /// is the scope of what follows. A block of comments on consecutive
    /// lines keeps the indentation of its lines relative to each other. A
    /// one-line `/* */` comment on the line of what follows stays in front
    /// of it.
    ///
    /// Returns whether the trivia ended the line, so that what follows
    /// starts a line whatever `sep` says.
    pub(crate) fn gap(&mut self, trivia: Range<usize>, sep: Sep, blank: u8) -> bool {
        let next = trivia.end;
        let whitespace = |i: usize| self.tokens[i].kind() == TokenKind::Whitespace;
        if trivia.clone().all(whitespace) {
            self.sep(sep);
            self.blank_before(next, blank);
            return false;
        }
        // A comment or tag can end the line.
        self.breaks += 1;
        let after_opener = trivia.start.checked_sub(1).is_some_and(|i| {
            matches!(
                self.tokens[i].kind(),
                TokenKind::OpeningRoundBracket | TokenKind::OpeningSquareBracket
            )
        });
        let closes = self.tokens.get(next).is_some_and(|t| {
            matches!(
                t.kind(),
                TokenKind::Comma
                    | TokenKind::Semicolon
                    | TokenKind::ClosingRoundBracket
                    | TokenKind::ClosingSquareBracket
            )
        });
        let solid: Vec<usize> = trivia.filter(|&i| !whitespace(i)).collect();
        let mut broke = false;
        let mut k = 0;
        // Comments that trail the line before: a `/* */` one is an atom,
        // spaced from what is around it but brackets and commas. A Jinja
        // comment is spaced from those too: one that touches a token keeps
        // its statement as written.
        let mut inline = None;
        while let Some(&i) = solid.get(k) {
            if !self.started || self.newlines_before(i) > 0 || self.is_tag(i) {
                break;
            }
            let kind = self.tokens[i].kind();
            if kind == TokenKind::LineComment {
                self.b.trailing_comment(i);
                k += 1;
                broke = true;
                inline = None;
                break;
            }
            if !(k == 0 && after_opener) || kind == TokenKind::JinjaComment {
                self.b.space();
            }
            self.b.token(i);
            inline = Some(kind);
            k += 1;
        }
        if let Some(kind) = inline
            && matches!(sep, Sep::Nothing | Sep::Soft)
            && (!closes || kind == TokenKind::JinjaComment)
            && k == solid.len()
        {
            self.b.space();
        }
        self.sep(sep);
        while let Some(&i) = solid.get(k) {
            self.blank_before(i, blank);
            if self.is_tag(i) {
                self.tag(i);
                broke = true;
                k += 1;
                continue;
            }
            // Everything here starts a line, also a comment that follows a
            // comment or tag on its line: the printer breaks the line after
            // those.
            // A one-line `/* */` comment on the line of what follows.
            let leads = |k: usize| {
                let i = solid[k];
                self.tokens[i].kind() != TokenKind::LineComment
                    && k + 1 == solid.len()
                    && self.newlines_before(next) == 0
                    && !self.text(i).contains('\n')
            };
            if leads(k) {
                // It starts its line, as in the input, so that what follows
                // on that line stays with it; a Jinja comment keeps the
                // whitespace after it, which once rendered is between what
                // comes before and after it.
                self.started = true;
                self.b.hard_line();
                self.b.token(i);
                let spaced = if self.tokens[i].kind() == TokenKind::JinjaComment {
                    self.tokens
                        .get(i + 1)
                        .is_some_and(|t| t.kind() == TokenKind::Whitespace)
                } else {
                    !closes
                };
                if spaced {
                    self.b.space();
                }
                broke = true;
                k += 1;
                continue;
            }
            // Comments on consecutive lines of their own.
            let mut end = k;
            while let Some(&j) = solid.get(end + 1) {
                if self.is_tag(j) || self.newlines_before(j) > 1 || leads(end + 1) {
                    break;
                }
                end += 1;
            }
            self.started = true;
            if end == k {
                self.b.own_line_comment(i);
            } else {
                self.b.comment_block(i, solid[end]);
            }
            broke = true;
            k = end + 1;
        }
        self.blank_before(next, blank);
        broke
    }

    /// Opens an indentation scope for the body of a clause if `trivia`, what
    /// comes between the clause's keywords and its body, ends the keywords'
    /// line: a line comment, or a comment or tag on a line of its own. The
    /// comments then go at the body's indentation and the body after them,
    /// rather than at the clause's. The caller closes the scope after the
    /// body.
    pub(crate) fn body_scope(&mut self, trivia: Range<usize>) -> Option<Open> {
        let ends_line = trivia.into_iter().any(|i| {
            let kind = self.tokens[i].kind();
            kind != TokenKind::Whitespace
                && (kind == TokenKind::LineComment
                    || kind == TokenKind::JinjaBlock
                    || self.newlines_before(i) > 0)
        });
        ends_line.then(|| self.b.open_indent())
    }

    /// Keeps up to `max` of the blank lines the input has before token
    /// `index`.
    pub(crate) fn blank_before(&mut self, index: usize, max: u8) {
        if max == 0 || index >= self.tokens.len() {
            return;
        }
        let blank = self.newlines_before(index).saturating_sub(1);
        let blank = u8::try_from(blank).unwrap_or(u8::MAX).min(max);
        if self.between_statements {
            self.b.blank_lines_at_brackets(blank);
        } else {
            self.b.blank_lines(blank);
        }
    }

    /// Whether token `index` is a Jinja block tag.
    pub(crate) fn is_tag(&self, index: usize) -> bool {
        self.tokens[index].kind() == TokenKind::JinjaBlock
    }

    /// Adds block tag `index` on a line of its own: an opener at the scope's
    /// indentation, and `{% else %}`, `{% elif %}` and closers at the
    /// indentation of their opener.
    pub(crate) fn tag(&mut self, index: usize) {
        self.started = true;
        match tag_role(self.text(index)) {
            TagRole::Open => {
                let id = self.b.block_tag(index);
                self.blocks.push(Some(id));
            }
            TagRole::Middle => match self.blocks.last() {
                Some(&Some(opener)) => {
                    self.b.block_tag_aligned(index, opener);
                }
                _ => {
                    self.b.block_tag(index);
                }
            },
            TagRole::Close => match self.blocks.pop() {
                Some(Some(opener)) => {
                    self.b.block_tag_aligned(index, opener);
                }
                _ => {
                    self.b.block_tag(index);
                }
            },
            TagRole::Other => {
                self.b.block_tag(index);
            }
        }
    }

    /// The block and comment tags that touch a token on a side without a
    /// `-` marker, and the tokens they touch: what must keep touching.
    pub(crate) fn touching_tags(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for (i, token) in self.tokens.iter().enumerate() {
            if !matches!(
                token.kind(),
                TokenKind::JinjaBlock | TokenKind::JinjaComment
            ) {
                continue;
            }
            let (before, after) = super::tree::touches(self.source, self.tokens, i);
            if before {
                out.extend([i - 1, i]);
            }
            if after {
                out.extend([i, i + 1]);
            }
        }
        out
    }

    /// Follows the blocks that tags printed as written open and close.
    fn track_tags(&mut self, first: usize, last: usize) {
        for i in first..=last {
            if !self.is_tag(i) {
                continue;
            }
            match tag_role(self.text(i)) {
                TagRole::Open => self.blocks.push(None),
                TagRole::Close => {
                    self.blocks.pop();
                }
                TagRole::Middle | TagRole::Other => {}
            }
        }
    }

    // Common shapes.

    /// Adds `parts`, children of a node of `parent` kind, on one line as
    /// far as their own rules allow, spaced as [`join`](Self::join) says. The trivia
    /// before the first part is the caller's.
    pub(crate) fn inline(&mut self, parent: SyntaxKind, parts: &[Part]) {
        self.inline_with(parent, parts, |_, _, _| None);
    }

    /// [`inline`](Self::inline), with `sep` choosing the separator between
    /// two parts where it returns one, and [`join`](Self::join) elsewhere.
    pub(crate) fn inline_with(
        &mut self,
        parent: SyntaxKind,
        parts: &[Part],
        sep: impl Fn(&Self, &El, &El) -> Option<Sep>,
    ) {
        for (k, part) in parts.iter().enumerate() {
            if k > 0 {
                let prev = &parts[k - 1].el;
                let sep =
                    sep(self, prev, &part.el).unwrap_or_else(|| self.join(parent, prev, &part.el));
                self.gap(part.trivia.clone(), sep, 0);
            }
            self.el(&part.el);
        }
    }

    /// Adds `node` by the generic rule: its children on one line, spaced as
    /// [`join`](Self::join) says.
    pub(crate) fn generic(&mut self, node: &Node) {
        let parts = self.parts(node);
        self.inline(node.kind(), &parts);
    }

    /// The space between two children of a node of `parent` kind that
    /// follow each other: none around `.` and `::`, inside brackets, before
    /// `,` and before the arguments of a call or a type; a space elsewhere.
    pub(crate) fn join(&self, parent: SyntaxKind, prev: &El, next: &El) -> Sep {
        use SyntaxKind as S;
        use TokenKind as T;
        let left = self.last_kind(prev);
        let right = self.first_kind(next);
        let none = matches!(
            right,
            T::Comma
                | T::Semicolon
                | T::ClosingRoundBracket
                | T::ClosingSquareBracket
                | T::ClosingCurlyBrace
                | T::Dot
                | T::DoubleColon
        ) || matches!(
            left,
            T::OpeningRoundBracket
                | T::OpeningSquareBracket
                | T::OpeningCurlyBrace
                | T::Dot
                | T::DoubleColon
                | T::DoubleAt
        );
        if none {
            return Sep::Nothing;
        }
        let glued = match (parent, next.kind()) {
            // `APPLY(f)`, but `EXCEPT (a, b)`.
            (_, S::ArgList | S::KeywordArgList | S::TypeArgList)
            | (S::FunctionCall, S::Subquery)
            | (S::ColumnsMatcher, S::ColumnList)
            | (S::QueryParameter, _)
            | (S::ColumnTransformer, S::OpeningRoundBracket) => true,
            (S::IndexExpr | S::DataType, _) => {
                matches!(right, T::OpeningSquareBracket | T::OpeningRoundBracket)
            }
            (S::PrefixExpr, _) => matches!(left, T::Minus | T::Plus),
            // `REFERENCES db.t(a)`: a table and its columns, written as a call.
            (S::ForeignKeyDef, _) => {
                right == T::OpeningRoundBracket && matches!(prev.kind(), S::Name | S::QualifiedName)
            }
            (S::MapEntry, _) => right == T::Colon,
            // JSON paths: `json.^a`, `json.a.:Int64`.
            (S::QualifiedName, _) => matches!(left, T::Caret | T::Colon),
            _ => false,
        };
        if glued { Sep::Nothing } else { Sep::Space }
    }

    /// Reserves a group, for a blank line before a statement that spans
    /// lines.
    pub(crate) fn reserve_group(&mut self) -> GroupId {
        self.b.reserve_group()
    }
}

/// What a block tag does to the blocks around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TagRole {
    /// `{% if %}`, `{% for %}` and the like.
    Open,
    /// `{% elif %}`, `{% else %}`.
    Middle,
    /// `{% endif %}` and the like.
    Close,
    /// `{% set x = 1 %}` and other tags that open nothing.
    Other,
}

/// The name of the block tag `text`, `if` for `{%- if x %}`, and what
/// follows it.
pub(crate) fn tag_name(text: &str) -> (&str, &str) {
    let inner = text
        .strip_prefix("{%")
        .unwrap_or(text)
        .trim_start_matches(['-', '+']);
    let inner = inner.trim_start();
    let name_len = inner
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(inner.len());
    inner.split_at(name_len)
}

/// What the block tag `text` does, by its name.
pub(crate) fn tag_role(text: &str) -> TagRole {
    let (name, rest) = tag_name(text);
    match name {
        "if" | "for" | "macro" | "call" | "filter" | "block" | "with" | "autoescape" | "trans" => {
            TagRole::Open
        }
        // `{% set x %}…{% endset %}` captures a block; `{% set x = 1 %}`
        // does not.
        "set" if !rest.contains('=') => TagRole::Open,
        "elif" | "else" | "pluralize" => TagRole::Middle,
        _ if name.starts_with("end") => TagRole::Close,
        _ => TagRole::Other,
    }
}
