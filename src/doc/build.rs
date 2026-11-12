//! Building a document.

use super::Doc;
use super::Ending;
use super::Gap;
use super::GroupId;
use super::TagId;
use super::bit;
use super::ir::Atom;
use super::ir::GroupInfo;
use super::ir::LineKind;
use super::ir::NONE;
use super::ir::Node;
use super::ir::Place;
use super::ir::Scope;
use super::ir::Sep;
use super::ir::Span;
use super::ir::Verbatim;
use super::ir::VerbatimInfo;
use super::ir::first_line;
use super::ir::narrow;
use super::print::line_comment_text;
use super::set_bit;
use super::touch::may_touch;
use crate::lexer::Mode;
use crate::lexer::Token;
use crate::lexer::TokenKind;

/// Builds a [`Doc`], node by node, in output order.
///
/// Tokens are named by their index in the token slice the builder was made
/// with (for the formatter, [`Split::tokens`](crate::split::Split::tokens),
/// which the syntax tree holds one for one). Every token other than
/// whitespace must be added exactly once, in order; [`Doc::check_tokens`]
/// checks that.
///
/// Scopes (groups and indentation) nest. The closure methods
/// ([`group`](Self::group), [`indent`](Self::indent), …) cannot leave one
/// open; the `open_*` methods return an [`Open`] that [`close`](Self::close)
/// takes back, for code that cannot nest closures. Closing out of order, or
/// finishing with a scope open, is a bug: it panics in debug builds, and in
/// release builds closes the scopes inside it too.
///
/// ```
/// use chairlift::doc::{Builder, PrintOptions};
/// use chairlift::lexer::{Mode, tokenize};
///
/// let source = "select a,b from t";
/// let tokens = tokenize(source, Mode::Sql);
/// let mut b = Builder::new(source, &tokens, Mode::Sql);
/// b.group_forced(|b| {
///     b.keyword(0); // select
///     b.indent(|b| {
///         b.line();
///         b.token(2); // a
///         b.token(3); // ,
///         b.line();
///         b.token(4); // b
///     });
/// });
/// b.hard_line();
/// b.keyword(6); // from
/// b.space();
/// b.token(8); // t
/// let doc = b.finish();
/// doc.check_tokens()?;
/// assert_eq!(doc.print(&PrintOptions::default()), "SELECT\n    a,\n    b\nFROM t\n");
/// # Ok::<(), chairlift::doc::TokenCheckError>(())
/// ```
pub struct Builder<'s> {
    doc: Doc<'s>,
    /// The open scopes, innermost last.
    scopes: Vec<ScopeKind>,
    /// The open groups, innermost last.
    groups: Vec<OpenGroup>,
    adj: Adjacency<'s>,
    any_frozen: bool,
    /// A bit per token: the last token of lines kept as written (see
    /// [`keep_lines`](Self::keep_lines)).
    line_ends: Vec<u64>,
    /// The first token that renders as text is a `Dot`, after tags that
    /// render nothing, and the rendered text has whitespace before it.
    dot_first: bool,
}

/// An open scope, to [`close`](Builder::close).
#[must_use = "a scope must be closed"]
#[derive(Debug)]
pub struct Open {
    depth: usize,
    group: Option<GroupId>,
}

impl Open {
    /// The group the scope is, if it is one.
    #[must_use]
    pub fn group(&self) -> Option<GroupId> {
        self.group
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopeKind {
    Group,
    Layout,
}

/// What the builder accumulates for a group until it closes.
struct OpenGroup {
    id: u32,
    span: Span,
    forced: bool,
    /// The earliest start of a group an [`Node::IfBreak`] inside refers to.
    min_ref: u32,
    frozen: bool,
}

/// What lies between the last token added and the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Between {
    /// Nothing: the tokens touch.
    Touching,
    /// Something that may print as nothing.
    Maybe,
    /// Something that always prints as whitespace.
    Separated,
    /// The input's whitespace.
    Frozen,
}

/// The last token added, for deciding how the next one joins it.
#[derive(Clone, Copy)]
struct Adjacency<'s> {
    last: Option<u32>,
    last_text: &'s str,
    /// The last significant token's kind before `last`.
    sig_before_last: Option<TokenKind>,
    /// The last significant token's kind up to `last`.
    last_sig: Option<TokenKind>,
    gap: Between,
}

/// The literal words printed in lower case.
const LOWER_LITERALS: [&str; 5] = ["true", "false", "inf", "infinity", "nan"];

impl<'s> Builder<'s> {
    /// A builder for a document over `tokens`, the tokens of `source` lexed
    /// in `mode`.
    #[must_use]
    pub fn new(source: &'s str, tokens: &'s [Token], mode: Mode) -> Builder<'s> {
        debug_assert!(
            tokens
                .last()
                .map_or(source.is_empty(), |t| t.end() == source.len()),
            "the tokens cover the source"
        );
        Builder {
            doc: Doc {
                source,
                tokens,
                mode,
                nodes: Vec::with_capacity(tokens.len() * 2),
                groups: Vec::new(),
                verbatims: Vec::new(),
                statics: Vec::new(),
                tags: Vec::new(),
                frozen: Vec::new(),
                line_starts: Vec::new(),
                ending: Ending::Newline,
            },
            scopes: Vec::new(),
            groups: Vec::new(),
            adj: Adjacency {
                last: None,
                last_text: "",
                sig_before_last: None,
                last_sig: None,
                gap: Between::Separated,
            },
            any_frozen: false,
            line_ends: Vec::new(),
            dot_first: dot_after_tags(source, tokens, mode),
        }
    }

    /// The text the document lays out.
    #[must_use]
    pub fn source(&self) -> &'s str {
        self.doc.source
    }

    /// The tokens the document refers to by index.
    #[must_use]
    pub fn tokens(&self) -> &'s [Token] {
        self.doc.tokens
    }

    /// The mode the tokens were lexed in.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.doc.mode
    }

    /// The number of line breaks in the whitespace just before token
    /// `index`: 0 for a comment that trails a line, 1 for one on its own
    /// line, 2 or more for one after a blank line.
    #[must_use]
    pub fn newlines_before(&self, index: usize) -> usize {
        index
            .checked_sub(1)
            .map(|i| self.doc.tokens[i])
            .filter(|t| t.kind() == TokenKind::Whitespace)
            .map_or(0, |t| {
                t.text(self.doc.source)
                    .bytes()
                    .filter(|&b| b == b'\n')
                    .count()
            })
    }

    /// The index of the token that starts at byte `offset`, if one does:
    /// how a [`SyntaxToken`](crate::syntax::SyntaxToken) is found among the
    /// tokens, by the start of its range.
    #[must_use]
    pub fn token_index_at(&self, offset: usize) -> Option<usize> {
        let tokens = self.doc.tokens;
        let i = tokens.partition_point(|t| t.start() < offset);
        (tokens.get(i)?.start() == offset).then_some(i)
    }

    /// Sets what follows the last token (by default [`Ending::Newline`]).
    pub fn set_ending(&mut self, ending: Ending) {
        self.doc.ending = ending;
    }

    // Atoms.

    /// Prints token `index` as written.
    pub fn token(&mut self, index: usize) {
        self.atom(index, None, Place::Inline, NONE);
    }

    /// Prints token `index`, a bare word, as `text`: the same word in another
    /// case, such as a keyword's upper case from
    /// [`Keyword::as_str`](crate::keyword::Keyword::as_str). The text is
    /// static, so re-casing allocates nothing.
    pub fn recased(&mut self, index: usize, text: &'static str) {
        let source = self.doc.tokens[index].text(self.doc.source);
        debug_assert!(
            source.eq_ignore_ascii_case(text) && source.len() == text.len(),
            "{source:?} re-cased as {text:?}"
        );
        if source == text {
            self.token(index);
        } else {
            self.atom(index, Some(text), Place::Inline, NONE);
        }
    }

    /// Prints token `index`, a keyword in a keyword slot, in upper case.
    pub fn keyword(&mut self, index: usize) {
        if let Some(keyword) = self.doc.tokens[index].keyword() {
            self.recased(index, keyword.as_str());
        } else {
            debug_assert!(false, "token {index} is not a keyword word");
            self.token(index);
        }
    }

    /// Prints token `index`, a literal word, in its canonical case: `true`,
    /// `false`, `inf`, `infinity` and `nan` in lower case, any other (such
    /// as `NULL`) in upper case.
    pub fn literal_word(&mut self, index: usize) {
        let text = self.doc.tokens[index].text(self.doc.source);
        match LOWER_LITERALS.iter().find(|w| w.eq_ignore_ascii_case(text)) {
            Some(lower) => self.recased(index, lower),
            None => self.keyword(index),
        }
    }

    /// Prints comment token `index` at the end of the line of the code
    /// before it, two spaces after it, with a line break after it: a comment
    /// that trails a line in the input. Whatever line break the layout puts
    /// before it is dropped. Where nothing is printed on the line yet, or
    /// the line ends in a line comment, it goes on a line of its own.
    ///
    /// It keeps every group around it from printing flat.
    pub fn trailing_comment(&mut self, index: usize) {
        self.atom(index, None, Place::Trailing, NONE);
    }

    /// Prints comment token `index` on a line of its own, at the
    /// indentation of the scope it is in: a comment on its own line in the
    /// input, which belongs to the code after it. A multi-line `/* */`
    /// comment is printed byte for byte from there.
    ///
    /// It keeps every group around it from printing flat.
    pub fn own_line_comment(&mut self, index: usize) {
        self.atom(index, None, Place::OwnLine, NONE);
    }

    /// Prints tokens `first` to `last`, comments on their own lines with
    /// only whitespace between them, as a block on lines of its own: the
    /// least indented line goes to the indentation of the scope, and the
    /// others keep their indentation relative to it.
    pub fn comment_block(&mut self, first: usize, last: usize) {
        self.verbatim_block(first, last, Verbatim::Block, true);
    }

    /// Prints Jinja block tag `index` (`{% … %}`) on a line of its own, at
    /// the indentation of the scope it is in.
    ///
    /// It keeps every group around it from printing flat.
    pub fn block_tag(&mut self, index: usize) -> TagId {
        self.tag(index, NONE)
    }

    /// Prints Jinja block tag `index`, such as `{% endif %}` or
    /// `{% else %}`, on a line of its own at the indentation of the line
    /// that `opener` was printed on, wherever the layout puts it.
    pub fn block_tag_aligned(&mut self, index: usize, opener: TagId) -> TagId {
        self.tag(index, opener.0)
    }

    /// Prints tokens `first` to `last` as written, re-indented as a block:
    /// whitespace on a line keeps its bytes, and each line after the first
    /// keeps its indentation relative to the least indented line of the
    /// block, which goes to the indentation of the line the block starts on.
    /// Blank lines collapse to one, and to none after `(` or before `)`;
    /// whitespace at the end of a line goes. The bytes of every token are
    /// kept, so lines that start inside a string, comment or other token
    /// are never touched.
    ///
    /// `first` and `last` must not be whitespace.
    pub fn verbatim(&mut self, first: usize, last: usize) {
        self.verbatim_block(first, last, Verbatim::Block, false);
    }

    /// Prints tokens `first` to `last` and the whitespace between them byte
    /// for byte: for text that ClickHouse reads as written, such as the left
    /// operand of `::`.
    ///
    /// `first` and `last` must not be whitespace.
    pub fn verbatim_exact(&mut self, first: usize, last: usize) {
        self.verbatim_block(first, last, Verbatim::Exact, false);
    }

    /// Prints tokens `first` to `last` with the lines they are on in the
    /// input: as [`verbatim`](Self::verbatim), except that every line after
    /// the first keeps the indentation it has in the input byte for byte,
    /// tabs included, wherever the first line is printed. This is the safe
    /// cleanup's treatment of a statement, for one the layout keeps as
    /// written.
    ///
    /// `first` and `last` must not be whitespace.
    pub fn as_written(&mut self, first: usize, last: usize) {
        self.verbatim_block(first, last, Verbatim::AsWritten, false);
    }

    // Layout.

    /// A space. Spaces are printed lazily: a space before a line break, or
    /// next to another space, is dropped.
    pub fn space(&mut self) {
        self.doc.nodes.push(Node::Space);
        self.content(|g| g.span.space());
        self.separated();
    }

    /// A space, or a line break where the enclosing group breaks.
    pub fn line(&mut self) {
        self.doc.nodes.push(Node::Line(LineKind::Normal));
        self.content(|g| g.span.space());
        self.separated();
    }

    /// Nothing, or a line break where the enclosing group breaks.
    pub fn soft_line(&mut self) {
        self.doc.nodes.push(Node::Line(LineKind::Soft));
        self.maybe_touching();
    }

    /// A line break. It keeps every group around it from printing flat.
    ///
    /// Line breaks at one place in the output do not add up: two hard lines
    /// in a row print one line break. Blank lines come only from
    /// [`blank_lines`](Self::blank_lines) and its relatives.
    pub fn hard_line(&mut self) {
        self.doc.nodes.push(Node::Line(LineKind::Hard));
        self.force_break();
        self.separated();
    }

    /// Keeps up to `n` blank lines here where a line break is printed here;
    /// the most any place in the output gets is two, and none goes right
    /// after `(` or right before `)`. Where no line break is printed, this
    /// prints nothing.
    pub fn blank_lines(&mut self, n: u8) {
        if n > 0 {
            self.doc.nodes.push(Node::BlankLines(n));
        }
    }

    /// As [`blank_lines`](Self::blank_lines), also right after `(` and
    /// right before `)`: for blank lines between statements, where one can
    /// end `INSERT` data that a statement starting with `)` follows.
    pub fn blank_lines_at_brackets(&mut self, n: u8) {
        if n > 0 {
            self.doc.nodes.push(Node::BlankLinesAtBrackets(n));
        }
    }

    /// A line break and one blank line (none after `(` or before `)`).
    pub fn blank_line(&mut self) {
        self.hard_line();
        self.blank_lines(1);
    }

    /// One blank line here, where a line break is printed here, if `group`
    /// breaks. `group` may be built after this: before a statement's group,
    /// this asks for a blank line before the statement if it prints on
    /// several lines. Where tokens come between this place and a group
    /// built after it, the blank line is printed if the group can never
    /// print flat, and otherwise not.
    pub fn blank_line_if_broken(&mut self, group: GroupId) {
        self.doc.nodes.push(Node::BlankIfBroken(group.0));
    }

    /// Prints `broken` here if `group` breaks, otherwise `flat`. `group` is
    /// one that encloses this place or was built before it.
    pub fn if_break(&mut self, group: GroupId, broken: Gap, flat: Gap) {
        let start = self.doc.groups[group.0 as usize].start;
        debug_assert!(start != NONE, "if_break refers to a group not built yet");
        self.doc.nodes.push(Node::IfBreak {
            group: group.0,
            broken,
            flat,
        });
        let forced = broken == Gap::HardLine || flat == Gap::HardLine;
        let spaced = matches!(flat, Gap::Space | Gap::Line);
        self.content(|g| {
            g.min_ref = g.min_ref.min(start);
            g.forced |= forced;
            if spaced {
                g.span.space();
            }
        });
        let separates = |gap| matches!(gap, Gap::Space | Gap::Line | Gap::HardLine);
        if separates(broken) && separates(flat) {
            self.separated();
        } else {
            self.maybe_touching();
        }
    }

    /// Keeps the innermost open group, and every group around it, from
    /// printing flat.
    pub fn force_break(&mut self) {
        self.content(|g| g.forced = true);
    }

    /// Prints the input's whitespace between the last token added and the
    /// next token, byte for byte, in place of whatever the layout puts
    /// there: for whitespace that is part of another token once a template
    /// is rendered, or that comes before a group which must not start with
    /// it. Where the tokens touched in the input, they touch. Before lines
    /// kept as written (see [`keep_lines`](Self::keep_lines)), it is their
    /// line break and indentation.
    pub fn frozen(&mut self) {
        let Some(last) = self.adj.last else {
            return;
        };
        let tokens = self.doc.tokens;
        let last = last as usize;
        if self.doc.is_line_start(last + 1) && !self.doc.is_frozen(last + 1) {
            self.push_indentation(last + 1);
            return;
        }
        let start = tokens[last].end();
        let end = match tokens.get(last + 1) {
            Some(t) if t.kind() == TokenKind::Whitespace => t.end(),
            _ => start,
        };
        self.push_frozen(start, end);
    }

    /// Freezes token `index`: if it is whitespace, the boundary it lies in
    /// is printed as in the input, byte for byte, as if
    /// [`frozen`](Self::frozen) were called there, and so is it inside
    /// [verbatim](Self::verbatim) text; two frozen tokens that touch in the
    /// input touch in the output, and a frozen line comment keeps the
    /// whitespace at its end. Call it before adding the tokens it concerns.
    ///
    /// In [`Mode::Jinja`], whitespace with a character the renderer does not
    /// count as whitespace (U+FEFF, U+200B and a few more, which stop a `-`
    /// marker from stripping) is frozen without being asked.
    pub fn freeze(&mut self, index: usize) {
        debug_assert!(
            self.adj.last.is_none_or(|last| index >= last as usize),
            "token {index} is frozen after the tokens after it were added"
        );
        set_bit(&mut self.doc.frozen, index);
        self.any_frozen = true;
    }

    /// Whether whitespace token `index` comes before lines kept as written
    /// (see [`keep_lines`](Self::keep_lines)).
    #[must_use]
    pub fn starts_kept_lines(&self, index: usize) -> bool {
        self.doc.is_line_start(index)
    }

    /// Keeps tokens `first` to `last`, whole lines of the input, as they
    /// are: the tokens and the whitespace between them are
    /// [frozen](Self::freeze), the first line starts a line with the
    /// input's indentation (and the blank lines the layout asks for before
    /// it), and a line break follows the last. `first` must start a line
    /// of the input and `last` end one, and neither may be whitespace. Call
    /// it before adding the tokens it concerns.
    ///
    /// Inside [verbatim](Self::verbatim) text, the line break before
    /// `first` keeps the input's indentation too.
    pub fn keep_lines(&mut self, first: usize, last: usize) {
        let tokens = self.doc.tokens;
        debug_assert!(
            first <= last
                && tokens[first].kind() != TokenKind::Whitespace
                && tokens[last].kind() != TokenKind::Whitespace,
            "lines kept as written start or end with whitespace: {first}..={last}"
        );
        for i in first..=last {
            self.freeze(i);
        }
        if let Some(ws) = first.checked_sub(1)
            && tokens[ws].kind() == TokenKind::Whitespace
            && tokens[ws].text(self.doc.source).contains('\n')
        {
            set_bit(&mut self.doc.line_starts, ws);
        }
        set_bit(&mut self.line_ends, last);
    }

    // Scopes.

    /// A group: printed flat (every [`line`](Self::line) a space, every
    /// [`soft_line`](Self::soft_line) nothing) if it fits on the line up to
    /// the next place the line can break after it, and broken (every line a
    /// line break) otherwise. A group inside a flat group is flat.
    pub fn group(&mut self, f: impl FnOnce(&mut Self)) -> GroupId {
        let id = self.new_group();
        let open = self.start_group(id);
        f(self);
        self.close(open);
        GroupId(id)
    }

    /// A group that never prints flat, such as a `SELECT` list with more
    /// than one item.
    pub fn group_forced(&mut self, f: impl FnOnce(&mut Self)) -> GroupId {
        self.group(|b| {
            b.force_break();
            f(b);
        })
    }

    /// A group id to build a group with later, for a
    /// [`blank_line_if_broken`](Self::blank_line_if_broken) before it.
    pub fn reserve_group(&mut self) -> GroupId {
        GroupId(self.new_group())
    }

    /// Builds the group `id` was [reserved](Self::reserve_group) for.
    pub fn group_reserved(&mut self, id: GroupId, f: impl FnOnce(&mut Self)) {
        let open = self.open_reserved_group(id);
        f(self);
        self.close(open);
    }

    /// Lines in the scope are indented one level more.
    pub fn indent(&mut self, f: impl FnOnce(&mut Self)) {
        self.scoped(Scope::Indent, f);
    }

    /// Lines in the scope are indented one level less.
    pub fn dedent(&mut self, f: impl FnOnce(&mut Self)) {
        self.scoped(Scope::Dedent, f);
    }

    /// Lines in the scope start at column 0.
    pub fn at_root(&mut self, f: impl FnOnce(&mut Self)) {
        self.scoped(Scope::Root, f);
    }

    /// Lines in the scope are indented as the line the scope starts on, so
    /// that a bracket that ends a line closes at that line's indentation:
    /// how a subquery's parentheses hug it.
    pub fn hug(&mut self, f: impl FnOnce(&mut Self)) {
        self.scoped(Scope::Hug, f);
    }

    /// Opens a [`group`](Self::group).
    pub fn open_group(&mut self) -> Open {
        let id = self.new_group();
        self.start_group(id)
    }

    /// Opens the group `id` was [reserved](Self::reserve_group) for.
    pub fn open_reserved_group(&mut self, id: GroupId) -> Open {
        debug_assert!(
            self.doc.groups[id.0 as usize].start == NONE,
            "group {} is built twice",
            id.0
        );
        self.start_group(id.0)
    }

    /// Opens an [`indent`](Self::indent) scope.
    pub fn open_indent(&mut self) -> Open {
        self.open_scope(Scope::Indent)
    }

    /// Opens a [`dedent`](Self::dedent) scope.
    pub fn open_dedent(&mut self) -> Open {
        self.open_scope(Scope::Dedent)
    }

    /// Opens an [`at_root`](Self::at_root) scope.
    pub fn open_at_root(&mut self) -> Open {
        self.open_scope(Scope::Root)
    }

    /// Opens a [`hug`](Self::hug) scope.
    pub fn open_hug(&mut self) -> Open {
        self.open_scope(Scope::Hug)
    }

    /// Closes the innermost open scope, which `open` must be.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "taking the handle ends its use, so a scope cannot be closed twice"
    )]
    pub fn close(&mut self, open: Open) {
        debug_assert_eq!(
            self.scopes.len(),
            open.depth,
            "a scope is closed while one inside it is open"
        );
        while self.scopes.len() >= open.depth && !self.scopes.is_empty() {
            self.close_innermost();
        }
    }

    /// The document.
    #[must_use]
    pub fn finish(mut self) -> Doc<'s> {
        debug_assert!(self.scopes.is_empty(), "a scope is left open");
        while !self.scopes.is_empty() {
            self.close_innermost();
        }
        debug_assert!(
            self.doc.groups.iter().all(|g| g.start != NONE),
            "a reserved group is never built"
        );
        self.doc
    }

    // Internals.

    fn content(&mut self, f: impl FnOnce(&mut OpenGroup)) {
        if let Some(group) = self.groups.last_mut() {
            f(group);
        }
    }

    fn separated(&mut self) {
        if self.adj.gap != Between::Frozen {
            self.adj.gap = Between::Separated;
        }
    }

    fn maybe_touching(&mut self) {
        if self.adj.gap == Between::Touching {
            self.adj.gap = Between::Maybe;
        }
    }

    fn new_group(&mut self) -> u32 {
        let id = narrow(self.doc.groups.len());
        self.doc.groups.push(GroupInfo {
            start: NONE,
            end: NONE,
            span: Span::default(),
            forced: false,
            exact: true,
        });
        id
    }

    fn start_group(&mut self, id: u32) -> Open {
        self.doc.groups[id as usize].start = narrow(self.doc.nodes.len());
        self.doc.nodes.push(Node::GroupStart(id));
        self.groups.push(OpenGroup {
            id,
            span: Span::default(),
            forced: false,
            min_ref: NONE,
            frozen: false,
        });
        self.scopes.push(ScopeKind::Group);
        Open {
            depth: self.scopes.len(),
            group: Some(GroupId(id)),
        }
    }

    fn open_scope(&mut self, scope: Scope) -> Open {
        self.doc.nodes.push(Node::ScopeStart(scope));
        self.scopes.push(ScopeKind::Layout);
        Open {
            depth: self.scopes.len(),
            group: None,
        }
    }

    fn scoped(&mut self, scope: Scope, f: impl FnOnce(&mut Self)) {
        let open = self.open_scope(scope);
        f(self);
        self.close(open);
    }

    fn close_innermost(&mut self) {
        match self.scopes.pop() {
            Some(ScopeKind::Group) => {
                let Some(group) = self.groups.pop() else {
                    return;
                };
                let end = narrow(self.doc.nodes.len());
                self.doc.nodes.push(Node::GroupEnd);
                let info = &mut self.doc.groups[group.id as usize];
                info.end = end;
                info.span = group.span;
                info.forced = group.forced;
                info.exact = !group.frozen && group.min_ref >= info.start;
                if let Some(parent) = self.groups.last_mut() {
                    parent.span.append(group.span);
                    parent.forced |= group.forced;
                    parent.min_ref = parent.min_ref.min(group.min_ref);
                    parent.frozen |= group.frozen;
                }
            }
            Some(ScopeKind::Layout) => self.doc.nodes.push(Node::ScopeEnd),
            None => {}
        }
    }

    fn tag(&mut self, index: usize, opener: u32) -> TagId {
        let id = narrow(self.doc.tags.len());
        self.doc.tags.push(opener);
        self.atom(index, None, Place::Tag, id);
        TagId(id)
    }

    fn atom(&mut self, index: usize, text: Option<&'static str>, place: Place, tag: u32) {
        let doc = &self.doc;
        let token = doc.tokens[index];
        let kind = token.kind();
        debug_assert!(kind != TokenKind::Whitespace, "token {index} is whitespace");
        let printed: &'s str = text.unwrap_or_else(|| token.text(doc.source));
        let measured = if kind == TokenKind::LineComment {
            line_comment_text(printed, doc.mode)
        } else {
            printed
        };
        let (width, multiline) = first_line(measured);
        let width = narrow(width);
        self.auto_freeze(index);
        let sep = self.sep(index, kind, printed, place);
        let aux = match (place, text) {
            (Place::Tag, _) => tag,
            (_, Some(text)) => {
                self.doc.statics.push(text);
                narrow(self.doc.statics.len() - 1)
            }
            (_, None) => NONE,
        };
        self.doc.nodes.push(Node::Atom(Atom {
            token: narrow(index),
            aux,
            width,
            sep,
            place,
            multiline,
        }));
        let breaks_after = kind == TokenKind::LineComment || place != Place::Inline;
        let forced = multiline || breaks_after;
        self.content(|g| {
            if forced {
                g.forced = true;
            } else {
                g.span.token(width, sep);
            }
        });
        self.advance(index, kind, printed);
        self.adj.gap = if breaks_after {
            Between::Separated
        } else {
            Between::Touching
        };
        self.end_kept_lines(index);
    }

    /// The width of the first line of verbatim tokens `first..=last` as the
    /// printer prints it, and whether they span lines. Outside exact text the
    /// printer drops the whitespace before a line break it may rewrite and at
    /// the end of a line comment, so the source's first line can be wider.
    fn verbatim_first_line(&self, first: usize, last: usize, kind: Verbatim) -> (usize, bool) {
        let tokens = self.doc.tokens;
        let source = self.doc.source;
        if kind == Verbatim::Exact {
            return first_line(&source[tokens[first].start()..tokens[last].end()]);
        }
        let mut total = 0;
        for (i, token) in tokens.iter().enumerate().take(last + 1).skip(first) {
            let text = token.text(source);
            let frozen = self.doc.is_frozen(i);
            let text = match token.kind() {
                TokenKind::Whitespace
                    if text.contains('\n')
                        && !frozen
                        && (self.doc.mode == Mode::Sql || renderer_whitespace(text)) =>
                {
                    return (total, true);
                }
                TokenKind::LineComment if !frozen => line_comment_text(text, self.doc.mode),
                _ => text,
            };
            let (width, newline) = first_line(text);
            total += width;
            if newline {
                return (total, true);
            }
        }
        (total, false)
    }

    fn verbatim_block(&mut self, first: usize, last: usize, kind: Verbatim, own_line: bool) {
        let tokens = self.doc.tokens;
        let source = self.doc.source;
        debug_assert!(first <= last && last < tokens.len(), "{first}..={last}");
        debug_assert!(
            tokens[first].kind() != TokenKind::Whitespace
                && tokens[last].kind() != TokenKind::Whitespace,
            "verbatim text starts or ends with whitespace"
        );
        self.auto_freeze(first);
        let place = if own_line {
            Place::OwnLine
        } else {
            Place::Inline
        };
        let sep = self.sep(
            first,
            tokens[first].kind(),
            tokens[first].text(source),
            place,
        );
        let (width, multiline) = self.verbatim_first_line(first, last, kind);
        let first_width = narrow(width);
        let breaks_after = own_line || tokens[last].kind() == TokenKind::LineComment;
        self.doc
            .nodes
            .push(Node::Verbatim(narrow(self.doc.verbatims.len())));
        self.doc.verbatims.push(VerbatimInfo {
            first: narrow(first),
            last: narrow(last),
            kind,
            own_line,
            sep,
            first_width,
            multiline,
        });
        let forced = multiline || breaks_after;
        self.content(|g| {
            if forced {
                g.forced = true;
            } else {
                g.span.token(first_width, sep);
            }
        });
        for (i, token) in tokens.iter().enumerate().take(last + 1).skip(first) {
            if token.kind() != TokenKind::Whitespace {
                self.advance(i, token.kind(), token.text(source));
            }
        }
        self.adj.gap = if breaks_after {
            Between::Separated
        } else {
            Between::Touching
        };
        self.end_kept_lines(last);
    }

    /// Records that token `index` is printed next.
    fn advance(&mut self, index: usize, kind: TokenKind, printed: &'s str) {
        let adj = &mut self.adj;
        adj.sig_before_last = adj.last_sig;
        if kind.is_significant() {
            adj.last_sig = Some(kind);
        }
        adj.last = Some(narrow(index));
        adj.last_text = printed;
    }

    /// How token `index` joins the last token added.
    fn sep(&self, index: usize, kind: TokenKind, printed: &str, place: Place) -> Sep {
        let tokens = self.doc.tokens;
        let Some(last) = self.adj.last else {
            // At the very start of a text, `.` begins a number: a `Dot` must
            // not get there, nor after tags there that render nothing, and a
            // number that starts with `.` there must stay there.
            return if kind == TokenKind::Dot || self.dot_first {
                Sep::Glue
            } else if tokens[index].start() == 0 && printed.starts_with('.') {
                Sep::Touch
            } else {
                Sep::Free
            };
        };
        let left = tokens[last as usize];
        let opaque = |kind| matches!(kind, TokenKind::JinjaExpr | TokenKind::Verbatim);
        if opaque(left.kind()) || opaque(kind) {
            return if left.end() == tokens[index].start() {
                Sep::Touch
            } else {
                Sep::Glue
            };
        }
        if place != Place::Inline {
            return Sep::Free;
        }
        match self.adj.gap {
            Between::Separated | Between::Frozen => Sep::Free,
            Between::Touching | Between::Maybe => {
                let left = (left.kind(), self.adj.last_text);
                if may_touch(
                    self.doc.mode,
                    self.adj.sig_before_last,
                    left,
                    (kind, printed),
                ) {
                    Sep::Free
                } else {
                    Sep::Glue
                }
            }
        }
    }

    /// Freezes the boundary before token `index` if the tokens there are
    /// frozen, or the whitespace there must stay in a template.
    fn auto_freeze(&mut self, index: usize) {
        let Some(last) = self.adj.last else {
            return;
        };
        if self.adj.gap == Between::Frozen {
            // Frozen already, with `frozen`.
            return;
        }
        let last = last as usize;
        let tokens = self.doc.tokens;
        let freeze = if index == last + 2 && tokens[last + 1].kind() == TokenKind::Whitespace {
            let ws = last + 1;
            (self.any_frozen && self.doc.is_frozen(ws))
                || (self.doc.mode == Mode::Jinja
                    && !renderer_whitespace(tokens[ws].text(self.doc.source)))
        } else {
            index == last + 1
                && self.any_frozen
                && self.doc.is_frozen(last)
                && self.doc.is_frozen(index)
        };
        if freeze {
            self.push_frozen(tokens[last].end(), tokens[index].start());
        } else if index == last + 2 && self.doc.is_line_start(last + 1) {
            self.push_indentation(last + 1);
        }
    }

    /// Starts a line with the input's indentation from whitespace token
    /// `ws` (see [`keep_lines`](Self::keep_lines)).
    fn push_indentation(&mut self, ws: usize) {
        let token = self.doc.tokens[ws];
        let text = token.text(self.doc.source);
        let start = token.start() + text.rfind('\n').map_or(0, |at| at + 1);
        self.doc.nodes.push(Node::Indentation {
            start: narrow(start),
            end: narrow(token.end()),
        });
        self.force_break();
        self.adj.gap = Between::Frozen;
    }

    /// Breaks the line after token `index`, just added, where it ends lines
    /// kept as written.
    fn end_kept_lines(&mut self, index: usize) {
        if bit(&self.line_ends, index) {
            self.hard_line();
        }
    }

    fn push_frozen(&mut self, start: usize, end: usize) {
        let newline = self.doc.source[start..end].contains('\n');
        self.doc.nodes.push(Node::Frozen {
            start: narrow(start),
            end: narrow(end),
        });
        self.content(|g| {
            g.frozen = true;
            g.forced |= newline;
        });
        self.adj.gap = Between::Frozen;
    }
}

/// Whether the first token of a template that renders as text is a `Dot`
/// after tags that render nothing, with whitespace before it once rendered:
/// a `.` that starts the rendered text is a number, and one after
/// whitespace a `Dot`, so a `Dot` must not start the printed text either.
fn dot_after_tags(source: &str, tokens: &[Token], mode: Mode) -> bool {
    let tags = |t: &&Token| matches!(t.kind(), TokenKind::JinjaBlock | TokenKind::JinjaComment);
    let first = tokens.iter().find(|t| t.kind() != TokenKind::Whitespace);
    mode == Mode::Jinja
        && first.is_some_and(|t| tags(&t))
        && tokens
            .iter()
            .find(|t| t.kind() != TokenKind::Whitespace && !tags(t))
            .is_some_and(|t| t.kind() == TokenKind::Dot)
        && !crate::verify::rendered(source).0.starts_with('.')
}

/// Whether the renderer strips all of `text` next to a `-` marker: whether
/// every character of it is whitespace to it, not only to ClickHouse.
pub(super) fn renderer_whitespace(text: &str) -> bool {
    text.is_ascii() || text.chars().all(char::is_whitespace)
}
