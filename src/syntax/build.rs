//! Building the tree from the parser's events.
//!
//! Every token of the text goes into the tree, in order. Significant tokens
//! come from `Token` events, with the kind the parser gave them; trivia is
//! placed by one rule (see [`syntax`](super#trivia)): pending trivia is
//! emitted into the innermost open node when the next node starts or the next
//! token is emitted, before either. So a node never begins or ends with
//! trivia; trivia lies between siblings, in the deepest node that holds the
//! significant tokens on both sides of it.

use rowan::GreenNode;
use rowan::GreenNodeBuilder;

use crate::lexer::Token;
use crate::syntax::SyntaxKind;
use crate::syntax::parser::Event;
use crate::verify::Facts;

/// The green tree of `tokens` (the tokens of `source`) as `events` arrange
/// them, and the facts the verifier needs about it (see
/// [`Parse::facts`](super::Parse::facts)).
///
/// Malformed events, which only a grammar bug produces, never lose a token:
/// a token the events skip is emitted where the events resume, unmatched
/// `Finish` events are ignored, and nodes left open are closed at the end.
pub(crate) fn build(source: &str, tokens: &[Token], mut events: Vec<Event>) -> (GreenNode, Facts) {
    let mut builder = Builder {
        source,
        tokens,
        green: GreenNodeBuilder::new(),
        facts: Facts::new(),
        next: 0,
        open: Vec::new(),
    };
    builder.green.start_node(SyntaxKind::Root.into());
    let mut parents = Vec::new();
    let tombstone = Event::Start {
        kind: None,
        forward_parent: 0,
    };
    for i in 0..events.len() {
        match std::mem::replace(&mut events[i], tombstone) {
            Event::Start {
                kind,
                forward_parent,
            } => {
                // A node that was preceded starts where its outermost wrapper
                // does: walk the chain of forward parents, then open them
                // outermost first.
                parents.push(kind);
                let mut at = i;
                let mut forward = forward_parent;
                while forward != 0 {
                    at += forward as usize;
                    let Some(event) = events.get_mut(at) else {
                        break;
                    };
                    match std::mem::replace(event, tombstone) {
                        Event::Start {
                            kind,
                            forward_parent,
                        } => {
                            parents.push(kind);
                            forward = forward_parent;
                        }
                        other => {
                            *event = other;
                            break;
                        }
                    }
                }
                if parents.iter().any(Option::is_some) {
                    builder.trivia();
                }
                for kind in parents.drain(..).rev().flatten() {
                    builder.start(kind);
                }
            }
            Event::Finish => builder.finish(),
            Event::Token { kind, index } => builder.token(kind, index as usize),
        }
    }
    while !builder.open.is_empty() {
        builder.finish();
    }
    while builder.next < tokens.len() {
        builder.emit(tokens[builder.next].kind().into());
    }
    builder.green.finish_node();
    (builder.green.finish(), builder.facts)
}

struct Builder<'a> {
    source: &'a str,
    tokens: &'a [Token],
    green: GreenNodeBuilder<'static>,
    facts: Facts,
    /// The index of the next token to emit.
    next: usize,
    /// The open nodes, with the index of the first token of each.
    open: Vec<(SyntaxKind, usize)>,
}

impl Builder<'_> {
    /// Emits the next token as `kind`.
    fn emit(&mut self, kind: SyntaxKind) {
        let token = self.tokens[self.next];
        if matches!(kind, SyntaxKind::Keyword | SyntaxKind::LiteralWord) {
            self.facts.recase(self.next);
        }
        self.green.token(kind.into(), token.text(self.source));
        self.next += 1;
    }

    /// Emits the trivia before the next significant token.
    fn trivia(&mut self) {
        while self.next < self.tokens.len() && self.tokens[self.next].kind().is_trivia() {
            self.emit(self.tokens[self.next].kind().into());
        }
    }

    fn start(&mut self, kind: SyntaxKind) {
        self.green.start_node(kind.into());
        self.open.push((kind, self.next));
    }

    fn finish(&mut self) {
        if let Some((kind, first)) = self.open.pop() {
            self.green.finish_node();
            if kind == SyntaxKind::LiteralAsText && self.next > first {
                self.facts.keep_verbatim(first, self.next - 1);
            }
        }
    }

    /// Emits everything up to the token at `index`, then that token as
    /// `kind`. A token that is behind already is not emitted again.
    fn token(&mut self, kind: SyntaxKind, index: usize) {
        while self.next < index.min(self.tokens.len()) {
            self.emit(self.tokens[self.next].kind().into());
        }
        if self.next == index && self.next < self.tokens.len() {
            self.emit(kind);
        }
    }
}
