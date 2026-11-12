//! Printing a document.

use std::cell::OnceCell;

use super::Doc;
use super::Ending;
use super::Gap;
use super::build::renderer_whitespace;
use super::ir::Atom;
use super::ir::LineKind;
use super::ir::Meter;
use super::ir::NONE;
use super::ir::Node;
use super::ir::Pend;
use super::ir::Place;
use super::ir::Scope;
use super::ir::Sep;
use super::ir::Verbatim;
use super::ir::first_line;
use super::ir::narrow;
use super::ir::width;
use crate::Options;
use crate::lexer::Mode;
use crate::lexer::Token;
use crate::lexer::TokenKind;
use crate::lexer::is_whitespace;
use crate::lexer::same_before_newline;
use crate::verify::render_joins_text;
use crate::verify::rendered;

/// How to print a document.
///
/// ```
/// use chairlift::doc::PrintOptions;
///
/// let options = PrintOptions::default().with_line_width(80);
/// assert_eq!((options.line_width, options.indent_width), (80, 4));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PrintOptions {
    /// The width lines are laid out to fit, in `char`s.
    pub line_width: usize,
    /// The columns one level of indentation adds.
    pub indent_width: usize,
}

impl PrintOptions {
    /// The style's indentation: four spaces.
    pub const DEFAULT_INDENT_WIDTH: usize = 4;

    /// These options with `line_width` set.
    #[must_use]
    pub fn with_line_width(mut self, line_width: usize) -> Self {
        self.line_width = line_width;
        self
    }

    /// These options with `indent_width` set.
    #[must_use]
    pub fn with_indent_width(mut self, indent_width: usize) -> Self {
        self.indent_width = indent_width;
        self
    }
}

impl Default for PrintOptions {
    fn default() -> Self {
        PrintOptions {
            line_width: usize::from(Options::DEFAULT_LINE_WIDTH),
            indent_width: Self::DEFAULT_INDENT_WIDTH,
        }
    }
}

impl From<&Options> for PrintOptions {
    fn from(options: &Options) -> Self {
        PrintOptions::default().with_line_width(usize::from(options.line_width))
    }
}

/// The most blank lines printed in one place.
const MAX_BLANK_LINES: u8 = 2;

/// A line comment's text as printed: without the whitespace at its end
/// (where a CRLF's `\r` sits), except that `# ` keeps its space, without
/// which it is not a comment. In a template, characters that ClickHouse
/// counts as whitespace and the renderer does not stay.
pub(super) fn line_comment_text(text: &str, mode: Mode) -> &str {
    let trimmed = text
        .trim_end_matches(|c: char| is_whitespace(c) && (mode == Mode::Sql || c.is_whitespace()));
    if trimmed == "#" { text } else { trimmed }
}

pub(super) fn print(doc: &Doc<'_>, options: &PrintOptions) -> String {
    let mut printer = Printer::new(doc, options);
    for i in 0..doc.nodes.len() {
        printer.node(i);
    }
    printer.finish()
}

/// Prints `doc` deciding every group by printing it flat and looking at the
/// line it makes: the definition the fast measurement must agree with.
#[cfg(test)]
pub(super) fn print_reference(doc: &Doc<'_>, options: &PrintOptions) -> String {
    let mut printer = Printer::new(doc, options);
    printer.measure = Measure::ByPrinting;
    for i in 0..doc.nodes.len() {
        printer.node(i);
    }
    printer.finish()
}

/// A line break waiting for the next token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Newline {
    #[default]
    None,
    /// From the layout: a trailing comment pulls the next token back up.
    Layout,
    /// After a line comment, an own-line comment or a block tag.
    Forced,
}

/// The whitespace waiting for the next token. Layout whitespace is printed
/// lazily, when the next token is, so that a space before a line break is
/// never printed and line breaks at one place do not add up.
#[derive(Clone, Copy, Debug)]
struct Pending {
    space: bool,
    newline: Newline,
    /// The indentation of the scope the last line break the layout asked for
    /// here is in; `None` where only a forced line break is pending, which
    /// takes the indentation of the next token's scope.
    indent: Option<usize>,
    blank: u8,
    /// Blank lines kept also next to brackets.
    blank_at_brackets: u8,
    /// A group whose breaking asks for a blank line, decided after this
    /// place was reached.
    blank_ref: u32,
    /// Input whitespace that replaces everything else here.
    frozen: Option<(u32, u32)>,
    /// The input's indentation, which the line a pending line break starts
    /// takes in place of the scope's.
    indentation: Option<(u32, u32)>,
}

impl Default for Pending {
    fn default() -> Self {
        Pending {
            space: false,
            newline: Newline::None,
            indent: None,
            blank: 0,
            blank_at_brackets: 0,
            blank_ref: NONE,
            frozen: None,
            indentation: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decision {
    Undecided,
    Flat,
    Broken,
}

/// How groups are measured.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Measure {
    Fast,
    /// By printing them, as `print_reference` does.
    ByPrinting,
}

/// A reference measurement in progress.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
struct Sim {
    stopped: bool,
    overflow: bool,
    /// The measurement starts a line: the next token's separation prints
    /// nothing.
    line_start: bool,
}

#[derive(Clone)]
struct Printer<'d, 's> {
    doc: &'d Doc<'s>,
    width: usize,
    indent_width: usize,
    out: String,
    /// The display width of the current line so far.
    column: usize,
    /// The indentation the current line started with.
    line_indent: usize,
    /// Something other than indentation is on the current line.
    line_has_content: bool,
    /// A token has been printed.
    started: bool,
    /// The first token is printed at the very start of the output, with
    /// nothing before it (see `Builder::sep`).
    starts_touching: bool,
    pending: Pending,
    /// For each open group, whether it prints flat.
    flat: Vec<bool>,
    /// The indentation of each open scope.
    indents: Vec<usize>,
    decisions: Vec<Decision>,
    /// The indentation each block tag was printed at.
    tag_indents: Vec<usize>,
    /// The kind of the last token printed.
    last_kind: Option<TokenKind>,
    /// The output offsets and indices of the last three tokens printed.
    tail: [(usize, u32); 3],
    tail_len: usize,
    /// The offset of each line of the source after the first, made when a
    /// verbatim block first needs the line it starts on.
    line_starts: OnceCell<Vec<usize>>,
    #[cfg(test)]
    measure: Measure,
    #[cfg(test)]
    sim: Option<Sim>,
}

impl<'d, 's> Printer<'d, 's> {
    fn new(doc: &'d Doc<'s>, options: &PrintOptions) -> Self {
        let starts_touching = doc
            .nodes
            .iter()
            .find_map(|node| match *node {
                Node::Atom(atom) => Some(atom.sep),
                Node::Verbatim(v) => Some(doc.verbatims[v as usize].sep),
                _ => None,
            })
            .is_some_and(|sep| sep == Sep::Touch);
        Printer {
            doc,
            width: options.line_width,
            indent_width: options.indent_width,
            out: String::with_capacity(doc.source.len() + doc.source.len() / 4 + 1),
            column: 0,
            line_indent: 0,
            line_has_content: false,
            started: false,
            starts_touching,
            pending: Pending::default(),
            flat: Vec::new(),
            indents: Vec::new(),
            decisions: vec![Decision::Undecided; doc.groups.len()],
            tag_indents: vec![usize::MAX; doc.tags.len()],
            last_kind: None,
            tail: [(0, 0); 3],
            tail_len: 0,
            line_starts: OnceCell::new(),
            #[cfg(test)]
            measure: Measure::Fast,
            #[cfg(test)]
            sim: None,
        }
    }

    fn node(&mut self, i: usize) {
        match self.doc.nodes[i] {
            Node::Atom(atom) => self.atom(atom),
            Node::Verbatim(v) => self.verbatim(v),
            Node::Space => self.pending.space = true,
            Node::Line(kind) => self.line(kind),
            Node::BlankLines(n) => self.pending.blank = self.pending.blank.max(n),
            Node::BlankLinesAtBrackets(n) => {
                self.pending.blank_at_brackets = self.pending.blank_at_brackets.max(n);
            }
            Node::BlankIfBroken(group) => match self.decisions[group as usize] {
                Decision::Broken => self.pending.blank = self.pending.blank.max(1),
                Decision::Flat => {}
                // A group that can never print flat breaks wherever it
                // starts, also after comments that come before it.
                Decision::Undecided if self.doc.groups[group as usize].forced => {
                    self.pending.blank = self.pending.blank.max(1);
                }
                Decision::Undecided => self.pending.blank_ref = group,
            },
            Node::IfBreak {
                group,
                broken,
                flat,
            } => {
                let gap = self.branch(group, broken, flat);
                self.gap(gap);
            }
            Node::Frozen { start, end } => {
                if self.pending.frozen.is_none() {
                    self.pending.frozen = Some((start, end));
                }
            }
            Node::Indentation { start, end } => {
                self.want_newline(Newline::Forced);
                self.pending.indentation = Some((start, end));
            }
            Node::GroupStart(group) => self.group_start(group),
            Node::GroupEnd => {
                self.flat.pop();
            }
            Node::ScopeStart(scope) => {
                let top = self.top_indent();
                let indent = match scope {
                    Scope::Indent => top + self.indent_width,
                    Scope::Dedent => top.saturating_sub(self.indent_width),
                    Scope::Root => 0,
                    Scope::Hug if !self.started => top,
                    Scope::Hug if self.pending.newline != Newline::None => self.pending_indent(),
                    Scope::Hug => self.line_indent,
                };
                self.indents.push(indent);
            }
            Node::ScopeEnd => {
                self.indents.pop();
            }
        }
    }

    fn top_indent(&self) -> usize {
        self.indents.last().copied().unwrap_or(0)
    }

    fn in_flat_group(&self) -> bool {
        self.flat.last().copied().unwrap_or(false)
    }

    fn line(&mut self, kind: LineKind) {
        match (kind, self.in_flat_group()) {
            (LineKind::Soft, true) => {}
            (LineKind::Normal, true) => self.pending.space = true,
            _ => self.want_newline(Newline::Layout),
        }
    }

    fn gap(&mut self, gap: Gap) {
        match gap {
            Gap::Nothing => {}
            Gap::Space => self.pending.space = true,
            Gap::SoftLine => self.line(LineKind::Soft),
            Gap::Line => self.line(LineKind::Normal),
            Gap::HardLine => self.line(LineKind::Hard),
        }
    }

    /// The branch of an [`Node::IfBreak`] on `group`; a group not decided yet
    /// counts as flat.
    fn branch(&self, group: u32, broken: Gap, flat: Gap) -> Gap {
        if self.decisions[group as usize] == Decision::Broken {
            broken
        } else {
            flat
        }
    }

    /// Asks for a line break before the next token. One the layout asks for
    /// is indented as the scope it is asked in, the last one asked for
    /// winning; a forced one leaves that to the next token's scope.
    fn want_newline(&mut self, level: Newline) {
        self.pending.newline = self.pending.newline.max(level);
        if level == Newline::Layout {
            self.pending.indent = Some(self.top_indent());
        }
        self.stop();
    }

    /// The indentation of the line a pending line break starts.
    fn pending_indent(&self) -> usize {
        self.pending.indent.unwrap_or_else(|| self.top_indent())
    }

    fn group_start(&mut self, group: u32) {
        // A line that starts with this group is indented as the group's
        // scope, which fixes where the group starts before it is measured.
        if (!self.started || self.pending.newline != Newline::None) && self.pending.indent.is_none()
        {
            self.pending.indent = Some(self.top_indent());
        }
        let flat = if self.in_flat_group() {
            true
        } else if self.doc.groups[group as usize].forced || self.measuring() {
            false
        } else {
            self.fits(group)
        };
        self.decisions[group as usize] = if flat {
            Decision::Flat
        } else {
            Decision::Broken
        };
        self.flat.push(flat);
    }

    fn atom(&mut self, atom: Atom) {
        let doc = self.doc;
        let kind = doc.kind(atom.token);
        if self.measuring() && (atom.place != Place::Inline || kind == TokenKind::LineComment) {
            self.stop();
            return;
        }
        match atom.place {
            Place::Inline => self.separate(atom.sep, kind, None),
            Place::Trailing => {
                if atom.sep != Sep::Touch
                    && self.pending.frozen.is_none()
                    && self.line_has_content
                    && self.pending.newline != Newline::Forced
                {
                    self.pending = Pending::default();
                    self.write_spaces(2);
                } else {
                    // A comment that cannot trail a line is placed as an
                    // own-line comment, which is what it is once printed.
                    self.own_line(atom.sep);
                    self.separate(atom.sep, kind, None);
                }
            }
            Place::OwnLine | Place::Tag => {
                self.own_line(atom.sep);
                let indent = if atom.place == Place::Tag {
                    self.opener_indent(atom.aux)
                } else {
                    None
                };
                self.separate(atom.sep, kind, indent);
                if atom.place == Place::Tag {
                    self.tag_indents[atom.aux as usize] = self.line_indent;
                }
            }
        }
        if self.stopped() {
            return;
        }
        let text = match (atom.place, atom.aux) {
            (Place::Inline, aux) if aux != NONE => doc.statics[aux as usize],
            _ => doc.token(atom.token).text(doc.source),
        };
        let text = if kind == TokenKind::LineComment && !doc.is_frozen(atom.token as usize) {
            line_comment_text(text, doc.mode)
        } else {
            text
        };
        let at = self.out.len();
        if atom.multiline || self.measuring() {
            self.write(text);
        } else {
            // The width was measured when the document was built.
            self.out.push_str(text);
            self.column += atom.width as usize;
        }
        self.printed(at, atom.token, kind);
        if kind == TokenKind::LineComment || atom.place != Place::Inline {
            self.want_newline(Newline::Forced);
        }
    }

    /// Before something that goes on a line of its own.
    fn own_line(&mut self, sep: Sep) {
        if sep != Sep::Touch && self.pending.frozen.is_none() && self.line_has_content {
            self.want_newline(Newline::Layout);
        }
    }

    fn opener_indent(&self, tag: u32) -> Option<usize> {
        let opener = self.doc.tags[tag as usize];
        if opener == NONE {
            return None;
        }
        let indent = self.tag_indents[opener as usize];
        (indent != usize::MAX).then_some(indent)
    }

    /// Prints what is pending before a token of `kind` that joins the token
    /// before it as `sep`; `indent` is the indentation if a line starts.
    fn separate(&mut self, sep: Sep, kind: TokenKind, indent: Option<usize>) {
        let line_indent = indent.unwrap_or_else(|| self.pending_indent());
        let pending = std::mem::take(&mut self.pending);
        #[cfg(test)]
        let line_start = self
            .sim
            .as_mut()
            .is_some_and(|sim| std::mem::take(&mut sim.line_start));
        if sep == Sep::Touch {
            return;
        }
        if let Some((start, end)) = pending.frozen {
            let text = &self.doc.source[start as usize..end as usize];
            self.write(text);
            if text.contains('\n') {
                self.line_has_content = false;
            }
            return;
        }
        #[cfg(test)]
        if line_start {
            return;
        }
        let indent = line_indent;
        if !self.started {
            if self.doc.source.starts_with('\u{feff}') && !self.measuring() {
                self.out.push('\u{feff}');
            }
            self.write_spaces(indent);
            self.line_indent = indent;
            if sep == Sep::Glue && indent == 0 {
                self.write_spaces(1);
            }
            return;
        }
        if pending.newline != Newline::None {
            let blank = if self.last_kind == Some(TokenKind::OpeningRoundBracket)
                || kind == TokenKind::ClosingRoundBracket
            {
                0
            } else {
                let referred = pending.blank_ref != NONE
                    && self.decisions[pending.blank_ref as usize] == Decision::Broken;
                pending.blank.max(u8::from(referred))
            };
            let blank = blank.max(pending.blank_at_brackets).min(MAX_BLANK_LINES);
            if let Some((start, end)) = pending.indentation {
                self.newline(usize::from(blank), 0);
                self.write(&self.doc.source[start as usize..end as usize]);
            } else {
                self.newline(usize::from(blank), indent);
            }
        } else if pending.space || sep == Sep::Glue {
            self.write_spaces(1);
        }
    }

    fn verbatim(&mut self, v: u32) {
        let doc = self.doc;
        let info = doc.verbatims[v as usize];
        if self.measuring() && info.own_line {
            self.stop();
            return;
        }
        let tokens = doc.tokens;
        let source = doc.source;
        let (first, last) = (info.first as usize, info.last as usize);
        if info.own_line {
            self.own_line(info.sep);
        }
        self.separate(info.sep, tokens[first].kind(), None);
        if self.stopped() {
            return;
        }
        if info.kind == Verbatim::Exact {
            let at = self.out.len();
            let start = tokens[first].start();
            self.write(&source[start..tokens[last].end()]);
            for (i, token) in tokens.iter().enumerate().take(last + 1).skip(first) {
                if token.kind() != TokenKind::Whitespace {
                    self.printed(at + token.start() - start, narrow(i), token.kind());
                }
            }
        } else {
            let base = self.line_indent;
            let common = self.common_indent(first, last);
            for (i, token) in tokens.iter().enumerate().take(last + 1).skip(first) {
                if self.stopped() {
                    return;
                }
                let text = token.text(source);
                match token.kind() {
                    TokenKind::Whitespace if self.rewritable(i, text) => {
                        let newlines = text.bytes().filter(|&b| b == b'\n').count();
                        if newlines == 0 {
                            self.write(text);
                        } else {
                            let blank = if tokens[i - 1].kind() == TokenKind::OpeningRoundBracket
                                || tokens[i + 1].kind() == TokenKind::ClosingRoundBracket
                            {
                                0
                            } else {
                                (newlines - 1).min(1)
                            };
                            let indentation = after_last_newline(text);
                            if info.kind == Verbatim::AsWritten || doc.is_line_start(i) {
                                self.newline(blank, 0);
                                self.write(indentation);
                                self.line_indent = indent_columns(indentation, self.indent_width);
                            } else {
                                let line = indent_columns(indentation, self.indent_width);
                                self.newline(blank, base + line.saturating_sub(common));
                            }
                        }
                    }
                    TokenKind::Whitespace => {
                        self.write(text);
                        if text.contains('\n') {
                            self.line_has_content = false;
                        }
                    }
                    kind => {
                        let text = if kind == TokenKind::LineComment && !doc.is_frozen(i) {
                            line_comment_text(text, doc.mode)
                        } else {
                            text
                        };
                        let at = self.out.len();
                        self.write(text);
                        self.printed(at, narrow(i), kind);
                    }
                }
            }
        }
        if info.own_line || tokens[last].kind() == TokenKind::LineComment {
            self.want_newline(Newline::Forced);
        }
    }

    /// Whether whitespace token `i` of a verbatim block may be re-indented.
    fn rewritable(&self, i: usize, text: &str) -> bool {
        !self.doc.is_frozen(i) && (self.doc.mode == Mode::Sql || renderer_whitespace(text))
    }

    /// The least indentation of the lines of a verbatim block: the line it
    /// starts on, and each line that starts in it at whitespace it may
    /// re-indent.
    fn common_indent(&self, first: usize, last: usize) -> usize {
        let doc = self.doc;
        let start = doc.tokens[first].start();
        let line_starts = self.line_starts.get_or_init(|| {
            doc.source
                .match_indices('\n')
                .map(|(at, _)| at + 1)
                .collect()
        });
        let line_start = match line_starts.partition_point(|&line| line <= start) {
            0 => 0,
            n => line_starts[n - 1],
        };
        let mut common = indent_columns(&doc.source[line_start..], self.indent_width);
        for (i, token) in doc.tokens.iter().enumerate().take(last + 1).skip(first) {
            let text = token.text(doc.source);
            if token.kind() == TokenKind::Whitespace
                && text.contains('\n')
                && self.rewritable(i, text)
                && !doc.is_line_start(i)
            {
                common = common.min(indent_columns(after_last_newline(text), self.indent_width));
            }
        }
        common
    }

    /// Records that token `index` of `kind` was printed at output offset
    /// `at`.
    fn printed(&mut self, at: usize, index: u32, kind: TokenKind) {
        self.line_has_content = true;
        self.started = true;
        self.last_kind = Some(kind);
        self.tail = [self.tail[1], self.tail[2], (at, index)];
        self.tail_len = (self.tail_len + 1).min(3);
    }

    fn write(&mut self, text: &str) {
        if self.measuring() {
            let (width, newline) = first_line(text);
            self.advance_column(width);
            if newline {
                self.stop();
            }
            return;
        }
        match text.rfind('\n') {
            None => self.column += width(text),
            Some(at) => {
                let last = &text[at + 1..];
                self.column = width(last);
                self.line_indent = indent_columns(last, self.indent_width);
            }
        }
        self.out.push_str(text);
    }

    fn write_spaces(&mut self, n: usize) {
        const SPACES: &str = "                                                                ";
        let mut left = n;
        while left > 0 && !self.measuring() {
            let chunk = left.min(SPACES.len());
            self.out.push_str(&SPACES[..chunk]);
            left -= chunk;
        }
        self.advance_column(n);
    }

    fn newline(&mut self, blank: usize, indent: usize) {
        if self.measuring() {
            self.stop();
            return;
        }
        for _ in 0..=blank {
            self.out.push('\n');
        }
        self.column = 0;
        self.write_spaces(indent);
        self.line_indent = indent;
        self.line_has_content = false;
    }

    fn finish(mut self) -> String {
        if self.tail_len == 0 {
            return String::new();
        }
        let doc = self.doc;
        let (_, last) = self.tail[2];
        match doc.ending {
            Ending::AsInput => {
                let end = doc.token(last).end();
                self.out.push_str(&doc.source[end..]);
            }
            Ending::BlankLine => self.out.push_str("\n\n"),
            Ending::Newline => {
                if self.newline_lexes_the_same() {
                    self.out.push('\n');
                }
            }
        }
        self.out
    }

    /// Whether the tokens at the end of the output lex the same with a
    /// final newline after them.
    fn newline_lexes_the_same(&self) -> bool {
        let doc = self.doc;
        let last = doc.token(self.tail[2].1);
        let text = last.text(doc.source);
        // Tokens followed by whitespace in the input lex the same before a
        // newline.
        let followed = last.end() < doc.source.len()
            || (last.kind() == TokenKind::LineComment
                && crate::verify::line_comment_text(text) != text);
        if followed {
            return true;
        }
        let (start, first) = self.tail[3 - self.tail_len];
        let prev_significant = doc.tokens[..first as usize]
            .iter()
            .rev()
            .find(|t| t.kind().is_significant())
            .map(|t: &Token| t.kind());
        let tail_same =
            same_before_newline(doc.mode, prev_significant, &self.out[start..], start > 0);
        tail_same
            && (doc.mode == Mode::Sql
                || !render_joins_text(&self.out)
                || same_before_newline(Mode::Jinja, None, &rendered(&self.out).0, false))
    }

    // Measuring.

    fn fits(&self, group: u32) -> bool {
        #[cfg(test)]
        if self.measure == Measure::ByPrinting {
            return self.fits_by_printing(group);
        }
        self.fits_fast(group)
    }

    /// Where the next token would start and what is pending before it.
    fn start_meter(&self) -> Meter {
        let pending = &self.pending;
        let frozen_pend = Pend::Frozen {
            width: 0,
            newline: false,
        };
        let (column, pend) = if let Some((start, end)) = pending.frozen {
            let text = &self.doc.source[start as usize..end as usize];
            match text.rfind('\n') {
                Some(at) => (width(&text[at + 1..]), frozen_pend),
                None => (self.column + width(text), frozen_pend),
            }
        } else if !self.started && self.starts_touching {
            (0, Pend::Nothing)
        } else if !self.started {
            let indent = self.pending_indent();
            let pend = if indent > 0 {
                Pend::LineStart
            } else {
                Pend::Start
            };
            (indent, pend)
        } else if pending.newline != Newline::None {
            let indent = match pending.indentation {
                Some((start, end)) => width(&self.doc.source[start as usize..end as usize]),
                None => self.pending_indent(),
            };
            (indent, Pend::LineStart)
        } else if pending.space {
            (self.column, Pend::Space)
        } else {
            (self.column, Pend::Nothing)
        };
        Meter {
            room: wide(self.width) - wide(column),
            pend,
        }
    }

    /// Whether `group` fits flat on the line, with what follows it up to the
    /// next place the line can break: Wadler's and Prettier's measure.
    ///
    /// A group is measured once, when the printer reaches it in a broken
    /// group. A group that holds no frozen whitespace and no
    /// [`Node::IfBreak`] looking outside it has its flat width from when it
    /// was built, so measuring it costs nothing; inside a group measured
    /// node by node, such groups are skipped the same way. What follows is
    /// measured as if every group in it breaks, up to its first line break
    /// or until the line is full; trailing comments are not measured.
    fn fits_fast(&self, group: u32) -> bool {
        let doc = self.doc;
        let info = doc.groups[group as usize];
        let mut meter = self.start_meter();
        if !meter.fits() {
            return false;
        }
        if info.exact {
            meter.span(info.span);
        } else {
            let mut i = info.start as usize + 1;
            while i < info.end as usize {
                match doc.nodes[i] {
                    Node::Atom(atom) => {
                        if atom.place != Place::Inline
                            || atom.multiline
                            || meter.token(atom.width, atom.sep)
                        {
                            return false;
                        }
                    }
                    Node::Verbatim(v) => {
                        let v = doc.verbatims[v as usize];
                        if v.multiline || v.own_line || meter.token(v.first_width, v.sep) {
                            return false;
                        }
                    }
                    Node::Space | Node::Line(LineKind::Normal) => meter.space(),
                    Node::Line(LineKind::Hard) | Node::Indentation { .. } => return false,
                    Node::IfBreak {
                        group: other,
                        broken,
                        flat,
                    } => {
                        let gap = if self.inside(other, group) {
                            flat
                        } else {
                            self.branch(other, broken, flat)
                        };
                        match gap {
                            Gap::Space | Gap::Line => meter.space(),
                            Gap::HardLine => return false,
                            Gap::Nothing | Gap::SoftLine => {}
                        }
                    }
                    Node::Frozen { start, end } => {
                        let (width, newline) = self.frozen_width(start, end);
                        meter.frozen(width, newline);
                    }
                    Node::GroupStart(inner) => {
                        let inner = doc.groups[inner as usize];
                        if inner.exact {
                            meter.span(inner.span);
                            i = inner.end as usize;
                        }
                    }
                    Node::Line(LineKind::Soft)
                    | Node::BlankLines(_)
                    | Node::BlankLinesAtBrackets(_)
                    | Node::BlankIfBroken(_)
                    | Node::GroupEnd
                    | Node::ScopeStart(_)
                    | Node::ScopeEnd => {}
                }
                if !meter.fits() {
                    return false;
                }
                i += 1;
            }
        }
        meter.fits() && self.rest_fits(info.end as usize + 1, group, meter)
    }

    /// Whether what follows a group measured flat, from node `from`, fits on
    /// the line up to its next line break.
    fn rest_fits(&self, from: usize, group: u32, mut meter: Meter) -> bool {
        let doc = self.doc;
        let start = from.min(doc.nodes.len());
        for (i, node) in doc.nodes.iter().enumerate().skip(start) {
            match *node {
                Node::Atom(atom) => {
                    if atom.place != Place::Inline || doc.kind(atom.token) == TokenKind::LineComment
                    {
                        return true;
                    }
                    if meter.token(atom.width, atom.sep) || atom.multiline {
                        return meter.fits();
                    }
                }
                Node::Verbatim(v) => {
                    let v = doc.verbatims[v as usize];
                    if v.own_line {
                        return true;
                    }
                    // A line comment at its end breaks the line after it.
                    if meter.token(v.first_width, v.sep)
                        || v.multiline
                        || doc.kind(v.last) == TokenKind::LineComment
                    {
                        return meter.fits();
                    }
                }
                Node::Space => meter.space(),
                Node::Line(_) | Node::Indentation { .. } => return true,
                Node::IfBreak {
                    group: other,
                    broken,
                    flat,
                } => {
                    let other_start = doc.groups[other as usize].start;
                    let gap = if self.inside(other, group) {
                        flat
                    } else if other_start != NONE
                        && other_start > doc.groups[group as usize].end
                        && (other_start as usize) < i
                    {
                        // A group in what follows, measured as broken.
                        broken
                    } else {
                        self.branch(other, broken, flat)
                    };
                    match gap {
                        Gap::Nothing => {}
                        Gap::Space => meter.space(),
                        Gap::SoftLine | Gap::Line | Gap::HardLine => return true,
                    }
                }
                Node::Frozen { start, end } => {
                    let (width, newline) = self.frozen_width(start, end);
                    meter.frozen(width, newline);
                }
                Node::BlankLines(_)
                | Node::BlankLinesAtBrackets(_)
                | Node::BlankIfBroken(_)
                | Node::GroupStart(_)
                | Node::GroupEnd
                | Node::ScopeStart(_)
                | Node::ScopeEnd => {}
            }
            if !meter.fits() {
                return false;
            }
        }
        true
    }

    /// Whether group `inner` lies inside group `outer` (or is it).
    fn inside(&self, inner: u32, outer: u32) -> bool {
        let groups = &self.doc.groups;
        let (inner, outer) = (groups[inner as usize], groups[outer as usize]);
        inner.start != NONE && inner.start >= outer.start && inner.start < outer.end
    }

    fn frozen_width(&self, start: u32, end: u32) -> (u32, bool) {
        let (width, newline) = first_line(&self.doc.source[start as usize..end as usize]);
        (narrow(width), newline)
    }

    // The reference measurement, compiled for tests only: elsewhere these
    // are constants the compiler folds away.

    #[cfg(not(test))]
    #[expect(clippy::unused_self, reason = "the test build measures")]
    fn measuring(&self) -> bool {
        false
    }

    #[cfg(not(test))]
    #[expect(clippy::unused_self, reason = "the test build measures")]
    fn stopped(&self) -> bool {
        false
    }

    #[cfg(not(test))]
    #[expect(clippy::unused_self, reason = "the test build measures")]
    fn stop(&mut self) {}

    #[cfg(not(test))]
    fn advance_column(&mut self, n: usize) {
        self.column += n;
    }

    #[cfg(test)]
    fn measuring(&self) -> bool {
        self.sim.is_some()
    }

    #[cfg(test)]
    fn stopped(&self) -> bool {
        self.sim.is_some_and(|sim| sim.stopped)
    }

    #[cfg(test)]
    fn stop(&mut self) {
        if let Some(sim) = &mut self.sim {
            sim.stopped = true;
        }
    }

    #[cfg(test)]
    fn advance_column(&mut self, n: usize) {
        self.column += n;
        if let Some(sim) = &mut self.sim
            && self.column > self.width
        {
            sim.overflow = true;
            sim.stopped = true;
        }
    }

    /// Whether `group` fits, by printing it flat, and what follows it with
    /// every group broken, until the line ends.
    #[cfg(test)]
    fn fits_by_printing(&self, group: u32) -> bool {
        let start = self.start_meter();
        if !start.fits() {
            return false;
        }
        let mut sim = self.clone();
        sim.out = String::new();
        sim.sim = Some(Sim::default());
        // A pending line break, or frozen whitespace with one, ends the line
        // before the group: start the measurement on the line after it, where
        // the fast measurement starts. Frozen whitespace stays pending, empty,
        // so that more of it at the same place is ignored, as when printing.
        let column = usize::try_from(wide(self.width) - start.room).unwrap_or(0);
        if let Some((start, end)) = self.pending.frozen {
            if self.doc.source[start as usize..end as usize].contains('\n') {
                sim.pending.frozen = Some((end, end));
                sim.column = column;
                sim.line_has_content = false;
            }
        } else if self.started && self.pending.newline != Newline::None {
            sim.pending = Pending::default();
            sim.column = column;
            sim.line_has_content = false;
            sim.sim = Some(Sim {
                line_start: true,
                ..Sim::default()
            });
        }
        sim.decisions[group as usize] = Decision::Flat;
        sim.flat.push(true);
        let start = self.doc.groups[group as usize].start as usize;
        for i in start + 1..self.doc.nodes.len() {
            if sim.stopped() {
                break;
            }
            sim.node(i);
        }
        !sim.sim.is_some_and(|sim| sim.overflow)
    }
}

fn wide(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// The text of a whitespace token after its last line break.
fn after_last_newline(text: &str) -> &str {
    text.rfind('\n').map_or(text, |at| &text[at + 1..])
}

/// The columns the whitespace at the start of `text` takes, with tab stops
/// every `tab` columns.
fn indent_columns(text: &str, tab: usize) -> usize {
    let tab = tab.max(1);
    let mut column = 0;
    for c in text.chars() {
        match c {
            '\t' => column = (column / tab + 1) * tab,
            '\n' | '\r' => break,
            c if is_whitespace(c) => column += 1,
            _ => break,
        }
    }
    column
}
