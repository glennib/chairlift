//! Random SQL with realistic nesting, rendered as untidy source text and
//! laid out by the style's rules, for the printer's benchmark and property
//! tests.
//!
//! The file is included with `#[path]`, with a module `api` in its parent
//! that re-exports the crate's `doc` and `lexer` modules (see
//! `doc_gen.rs`). The layout walks the same syntax as the rendering, token
//! by token, which is what a layout over the real syntax tree does.

#![allow(dead_code, reason = "each includer uses part of it")]

use super::api::doc::Builder;
use super::api::doc::Doc;
use super::api::lexer::Mode;
use super::api::lexer::Token;
use super::api::lexer::TokenKind;

/// A small deterministic random number generator (SplitMix64).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number below `n` (`n` > 0).
    pub fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap_or(0)
    }

    /// True `percent` times in a hundred.
    pub fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

pub enum Expr {
    Name(&'static str),
    Qualified(&'static str, &'static str),
    Number(u32),
    Str(&'static str),
    Call(&'static str, Vec<Expr>),
    Binary(Box<Expr>, &'static str, Box<Expr>),
    Case(Vec<(Expr, Expr)>, Box<Expr>),
    Lambda(Vec<&'static str>, Box<Expr>),
    Tuple(Vec<Expr>),
    In(Box<Expr>, Box<Query>),
}

pub enum Comment {
    None,
    Trailing,
    OwnLine,
}

pub struct Item {
    pub expr: Expr,
    pub alias: Option<&'static str>,
    pub comment: Comment,
}

pub enum From {
    Table(&'static str),
    Subquery(Box<Query>, &'static str),
}

pub struct Query {
    pub items: Vec<Item>,
    pub from: From,
    pub conditions: Vec<Expr>,
    pub group_by: Vec<Expr>,
}

const NAMES: &[&str] = &[
    "site",
    "event_time",
    "visitor_id",
    "views",
    "referrer_domain",
    "event_type",
    "duration_ms",
    "status",
    "a",
    "b",
    "payload_product_feature",
];
const FUNCTIONS: &[&str] = &[
    "count",
    "sum",
    "toStartOfHour",
    "uniqState",
    "countIf",
    "nullIf",
    "round",
    "arraySlice",
    "groupUniqArray",
    "toDate",
    "cityHash64",
    "arrayStringConcat",
];
const TABLES: &[&str] = &["raw.events", "analytics.page_views_1h", "dim.prices"];
const STRINGS: &[&str] = &[
    "'page_view'",
    "'subscribe'",
    "''",
    "'https://internal.example.com/'",
];
const OPERATORS: &[&str] = &["+", "-", "*", "/", "=", ">=", "<", "!="];
const ALIASES: &[&str] = &[
    "hour",
    "views",
    "total",
    "conversion_pct",
    "top_referrer_domains",
];

pub fn query(rng: &mut Rng, depth: usize) -> Query {
    let items = (0..=rng.below(6))
        .map(|_| Item {
            expr: expr(rng, depth),
            alias: rng.chance(40).then(|| rng.pick(ALIASES)),
            comment: match rng.below(12) {
                0 => Comment::Trailing,
                1 => Comment::OwnLine,
                _ => Comment::None,
            },
        })
        .collect();
    let from = if depth < 2 && rng.chance(15) {
        From::Subquery(Box::new(query(rng, depth + 1)), "daily")
    } else {
        From::Table(rng.pick(TABLES))
    };
    Query {
        items,
        from,
        conditions: (0..rng.below(4)).map(|_| expr(rng, depth + 1)).collect(),
        group_by: (0..rng.below(4))
            .map(|_| Expr::Name(rng.pick(NAMES)))
            .collect(),
    }
}

pub fn expr(rng: &mut Rng, depth: usize) -> Expr {
    let leaf = depth > 3 || rng.chance(35);
    if leaf {
        return match rng.below(5) {
            0 => Expr::Number(u32::try_from(rng.below(100_000)).unwrap_or(0)),
            1 => Expr::Str(rng.pick(STRINGS)),
            2 => Expr::Qualified("o", rng.pick(NAMES)),
            _ => Expr::Name(rng.pick(NAMES)),
        };
    }
    match rng.below(10) {
        0..=3 => Expr::Call(
            rng.pick(FUNCTIONS),
            (0..rng.below(4)).map(|_| expr(rng, depth + 1)).collect(),
        ),
        4 | 5 => Expr::Binary(
            Box::new(expr(rng, depth + 1)),
            rng.pick(OPERATORS),
            Box::new(expr(rng, depth + 1)),
        ),
        6 => Expr::Case(
            (0..=rng.below(3))
                .map(|_| (expr(rng, depth + 1), expr(rng, depth + 1)))
                .collect(),
            Box::new(expr(rng, depth + 1)),
        ),
        7 => Expr::Lambda(vec!["x"], Box::new(expr(rng, depth + 1))),
        8 => Expr::Tuple(
            (0..2 + rng.below(3))
                .map(|_| expr(rng, depth + 1))
                .collect(),
        ),
        _ if depth < 2 => Expr::In(
            Box::new(Expr::Name(rng.pick(NAMES))),
            Box::new(query(rng, depth + 2)),
        ),
        _ => Expr::Name(rng.pick(NAMES)),
    }
}

/// Writes tokens with untidy whitespace between them.
pub struct Writer<'r> {
    pub out: String,
    rng: &'r mut Rng,
}

impl<'r> Writer<'r> {
    pub fn new(rng: &'r mut Rng) -> Self {
        Writer {
            out: String::new(),
            rng,
        }
    }

    fn token(&mut self, text: &str) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push_str(match self.rng.below(10) {
                0 => "\n  ",
                1 => "  ",
                _ => " ",
            });
        }
        self.out.push_str(text);
    }

    fn line_comment(&mut self, own_line: bool) {
        self.out.push_str(if own_line { "\n" } else { " " });
        self.out.push_str("-- a note on this item");
        self.out.push('\n');
    }

    pub fn query(&mut self, q: &Query) {
        self.token("select");
        for (k, item) in q.items.iter().enumerate() {
            if k > 0 {
                self.token(",");
                if matches!(q.items[k - 1].comment, Comment::Trailing) {
                    self.line_comment(false);
                }
            }
            if matches!(item.comment, Comment::OwnLine) {
                self.line_comment(true);
            }
            self.expr(&item.expr);
            if let Some(alias) = item.alias {
                self.token("as");
                self.token(alias);
            }
        }
        self.token("from");
        match &q.from {
            From::Table(t) => self.token(t),
            From::Subquery(sub, alias) => {
                self.token("(");
                self.query(sub);
                self.token(")");
                self.token("as");
                self.token(alias);
            }
        }
        for (k, c) in q.conditions.iter().enumerate() {
            self.token(if k == 0 { "where" } else { "and" });
            self.expr(c);
        }
        for (k, g) in q.group_by.iter().enumerate() {
            if k == 0 {
                self.token("group");
                self.token("by");
            } else {
                self.token(",");
            }
            self.expr(g);
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Name(n) => self.token(n),
            Expr::Qualified(q, n) => self.token(&format!("{q}.{n}")),
            Expr::Number(n) => self.token(&n.to_string()),
            Expr::Str(s) => self.token(s),
            Expr::Call(f, args) => {
                self.token(f);
                self.out.push('(');
                for (k, a) in args.iter().enumerate() {
                    if k > 0 {
                        self.token(",");
                    }
                    self.expr(a);
                }
                self.token(")");
            }
            Expr::Binary(l, op, r) => {
                self.token("(");
                self.expr(l);
                self.token(op);
                self.expr(r);
                self.token(")");
            }
            Expr::Case(arms, otherwise) => {
                self.token("case");
                for (w, t) in arms {
                    self.token("when");
                    self.expr(w);
                    self.token("then");
                    self.expr(t);
                }
                self.token("else");
                self.expr(otherwise);
                self.token("end");
            }
            Expr::Lambda(params, body) => {
                self.token(params[0]);
                self.token("->");
                self.expr(body);
            }
            Expr::Tuple(items) => {
                self.token("(");
                for (k, i) in items.iter().enumerate() {
                    if k > 0 {
                        self.token(",");
                    }
                    self.expr(i);
                }
                self.token(")");
            }
            Expr::In(l, q) => {
                self.expr(l);
                self.token("in");
                self.token("(");
                self.query(q);
                self.token(")");
            }
        }
    }
}

/// Statements of random queries, `;`-terminated.
pub fn script(rng: &mut Rng, statements: usize) -> (Vec<Query>, String) {
    let queries: Vec<Query> = (0..statements).map(|_| query(rng, 0)).collect();
    let mut w = Writer::new(rng);
    for q in &queries {
        w.query(q);
        w.token(";");
        w.out.push_str("\n\n");
    }
    let source = w.out;
    (queries, source)
}

/// Lays out `queries`, whose tokens are `tokens` of `source`.
pub fn document<'s>(
    queries: &[Query],
    source: &'s str,
    tokens: &'s [Token],
    mode: Mode,
) -> Doc<'s> {
    let mut l = Layout {
        b: Builder::new(source, tokens, mode),
        tokens,
        source,
        at: 0,
    };
    let mut previous = None;
    for q in queries {
        let id = l.b.reserve_group();
        if let Some(previous) = previous {
            l.b.hard_line();
            let blank = l.b.newlines_before(l.peek()).saturating_sub(1).min(2);
            l.b.blank_lines(u8::try_from(blank).unwrap_or(2));
            l.b.blank_line_if_broken(previous);
            l.b.blank_line_if_broken(id);
        }
        let open = l.b.open_reserved_group(id);
        l.query(q);
        l.b.close(open);
        l.token(";");
        previous = Some(id);
    }
    l.b.finish()
}

struct Layout<'s> {
    b: Builder<'s>,
    tokens: &'s [Token],
    source: &'s str,
    at: usize,
}

impl Layout<'_> {
    fn peek(&self) -> usize {
        let mut i = self.at;
        while self.tokens[i].kind() == TokenKind::Whitespace {
            i += 1;
        }
        i
    }

    fn take(&mut self, text: &str) -> usize {
        let i = self.peek();
        debug_assert!(
            self.tokens[i].text(self.source).eq_ignore_ascii_case(text),
            "token {i} {:?} is not {text:?}",
            self.tokens[i].text(self.source)
        );
        self.at = i + 1;
        i
    }

    fn token(&mut self, text: &str) {
        let i = self.take(text);
        self.b.token(i);
    }

    fn keyword(&mut self, text: &str) {
        let i = self.take(text);
        self.b.keyword(i);
    }

    fn comment(&mut self) {
        let i = self.peek();
        if self.tokens[i].kind() == TokenKind::LineComment {
            self.at = i + 1;
            if self.b.newlines_before(i) == 0 {
                self.b.trailing_comment(i);
            } else {
                self.b.own_line_comment(i);
            }
        }
    }

    fn query(&mut self, q: &Query) {
        let open = self.b.open_group();
        if q.items.len() > 1 {
            self.b.force_break();
        }
        self.keyword("select");
        let indent = self.b.open_indent();
        for (k, item) in q.items.iter().enumerate() {
            if k > 0 {
                self.token(",");
                self.comment();
            }
            self.b.line();
            self.comment();
            self.expr(&item.expr);
            if let Some(alias) = item.alias {
                self.b.space();
                self.keyword("as");
                self.b.space();
                self.token(alias);
            }
        }
        self.b.close(indent);
        self.b.close(open);
        self.b.hard_line();
        self.keyword("from");
        self.b.space();
        match &q.from {
            From::Table(t) => self.dotted(t),
            From::Subquery(sub, alias) => {
                self.hugged(sub);
                self.b.space();
                self.keyword("as");
                self.b.space();
                self.token(alias);
            }
        }
        if let Some((first, rest)) = q.conditions.split_first() {
            self.b.hard_line();
            self.keyword("where");
            self.b.space();
            self.b.group(|_| {});
            let open = self.b.open_group();
            self.expr(first);
            let indent = self.b.open_indent();
            for c in rest {
                self.b.line();
                self.keyword("and");
                self.b.space();
                self.expr(c);
            }
            self.b.close(indent);
            self.b.close(open);
        }
        if !q.group_by.is_empty() {
            self.b.hard_line();
            self.keyword("group");
            self.b.space();
            self.keyword("by");
            let open = self.b.open_group();
            let indent = self.b.open_indent();
            for (k, g) in q.group_by.iter().enumerate() {
                if k > 0 {
                    self.token(",");
                }
                self.b.line();
                self.expr(g);
            }
            self.b.close(indent);
            self.b.close(open);
        }
    }

    fn hugged(&mut self, q: &Query) {
        self.token("(");
        let hug = self.b.open_hug();
        let indent = self.b.open_indent();
        self.b.hard_line();
        self.query(q);
        self.b.close(indent);
        self.b.hard_line();
        self.token(")");
        self.b.close(hug);
    }

    fn dotted(&mut self, name: &str) {
        for (k, part) in name.split('.').enumerate() {
            if k > 0 {
                self.token(".");
            }
            self.token(part);
        }
    }

    /// A bracketed list: one line if it fits, else one item per line.
    fn list(&mut self, items: &[Expr]) {
        self.token("(");
        if !items.is_empty() {
            let open = self.b.open_group();
            let indent = self.b.open_indent();
            self.b.soft_line();
            for (k, e) in items.iter().enumerate() {
                if k > 0 {
                    self.token(",");
                    self.b.line();
                }
                self.expr(e);
            }
            self.b.close(indent);
            self.b.soft_line();
            self.b.close(open);
        }
        self.token(")");
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Name(n) => self.token(n),
            Expr::Qualified(q, n) => {
                self.token(q);
                self.token(".");
                self.token(n);
            }
            Expr::Number(_) => {
                let i = self.peek();
                self.at = i + 1;
                self.b.token(i);
            }
            Expr::Str(s) => self.token(s),
            Expr::Call(f, args) => {
                self.token(f);
                self.list(args);
            }
            Expr::Binary(l, op, r) => {
                self.token("(");
                let open = self.b.open_group();
                self.expr(l);
                let indent = self.b.open_indent();
                self.b.line();
                self.token(op);
                self.b.space();
                self.expr(r);
                self.b.close(indent);
                self.b.close(open);
                self.token(")");
            }
            Expr::Case(arms, otherwise) => {
                let open = self.b.open_group();
                self.keyword("case");
                let indent = self.b.open_indent();
                for (w, t) in arms {
                    self.b.line();
                    self.keyword("when");
                    self.b.space();
                    self.expr(w);
                    self.b.space();
                    self.keyword("then");
                    self.b.space();
                    self.expr(t);
                }
                self.b.line();
                self.keyword("else");
                self.b.space();
                self.expr(otherwise);
                self.b.close(indent);
                self.b.line();
                self.keyword("end");
                self.b.close(open);
            }
            Expr::Lambda(params, body) => {
                let open = self.b.open_group();
                self.token(params[0]);
                self.b.space();
                self.token("->");
                let indent = self.b.open_indent();
                self.b.line();
                self.expr(body);
                self.b.close(indent);
                self.b.close(open);
            }
            Expr::Tuple(items) => self.list(items),
            Expr::In(l, q) => {
                self.expr(l);
                self.b.space();
                self.keyword("in");
                self.b.space();
                self.hugged(q);
            }
        }
    }
}
