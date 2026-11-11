//! Rendering a template once per combination of its branches, to check
//! that formatting kept every render's SQL: a harness for tests and the
//! corpus tool, behind the `template` feature.
//!
//! The [verifier](crate::verify) checks one render of a template, the one
//! with every branch taken. [`render`] renders a template in many: it
//! replaces the condition of every `{% if %}` and `{% elif %}` tag by a
//! choice it makes, so that each combination of branches renders once (up
//! to [`MAX_COMBINATIONS`], sampled beyond), and the iterable of every
//! `{% for %}` loop by the first 0, 1 and 3 of its items
//! ([`LOOP_COUNTS`]). Formatting changes no tag, so the tags of a template
//! and of its formatted output are the same and in the same order, and the
//! variants of the two pair up: [`compare_sql`] then says whether a pair
//! holds the same SQL.
//!
//! The environment is chum's: the default delimiters, `trim_blocks` and
//! `lstrip_blocks` off, `keep_trailing_newline` on, strict undefined
//! behaviour, no auto-escaping, and chum's `sql_*` filters as stubs that
//! quote what they are given. Every name a template uses without defining
//! it is a stub: it renders as an identifier spelled after its path
//! (`env.db` renders `env_db`), any attribute of it or call to it is
//! another stub, and a loop over it runs as many times as the variant
//! says. dbt's `config()` renders nothing, `ref('x')` renders `x`,
//! `source('s', 't')` renders `s.t`, and `is_incremental()` is false, so
//! that dbt models render to SQL ClickHouse can read. Macros, `{% set %}`,
//! loops over literal lists and the builtin filters run as minijinja runs
//! them.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use minijinja::AutoEscape;
use minijinja::Environment;
use minijinja::Error as RenderError;
use minijinja::State;
use minijinja::UndefinedBehavior;
use minijinja::Value;
use minijinja::syntax::SyntaxConfig;
use minijinja::value::Enumerator;
use minijinja::value::Kwargs;
use minijinja::value::Object;
use minijinja::value::ObjectRepr;
use minijinja::value::Rest;

use crate::keyword::Keyword;
use crate::lexer::Mode;
use crate::lexer::TokenKind;
use crate::lexer::jinja;
use crate::lexer::jinja::TagKind;
use crate::lexer::jinja::WhitespaceControl;

/// The most combinations of branches rendered per template; a template with
/// more renders this many, sampled.
pub const MAX_COMBINATIONS: usize = 16;

/// How many times each `{% for %}` loop runs, one variant each.
pub const LOOP_COUNTS: [usize; 3] = [0, 1, 3];

/// The fuel one render may spend (see minijinja's `fuel` feature), which
/// bounds the work of a template that loops without end.
const FUEL: u64 = 50_000_000;

/// The branches and loops of a template that [`render`] controls.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Branches {
    /// For each chain of `{% if %}` and `{% elif %}` tags, in the order the
    /// chains start, how many branches it can take: one per condition, and
    /// one for its `{% else %}` or for none at all.
    pub chains: Vec<usize>,
    /// How many `{% for %}` loops run as many times as the variant says.
    pub loops: usize,
}

/// A template rendered in each of its variants.
#[derive(Clone, Debug)]
pub struct Rendering {
    /// The branches and loops found.
    pub branches: Branches,
    /// How many combinations of branches there are, saturating.
    pub combinations: u64,
    /// Whether the combinations rendered are a sample of them.
    pub sampled: bool,
    /// One per combination of branches rendered and loop count, in an order
    /// that depends only on [`branches`](Self::branches).
    pub variants: Vec<Variant>,
}

/// One render of a template.
#[derive(Clone, Debug)]
pub struct Variant {
    /// The branch each chain takes and how often loops run, as
    /// `branches 0.1.0, loops 3`.
    pub label: String,
    /// The rendered text, or the renderer's error.
    pub text: Result<String, String>,
}

/// Renders `source`, a template, once per variant; see the
/// [module documentation](self). A template that does not compile has one
/// variant, its error.
#[must_use]
pub fn render(source: &str) -> Rendering {
    let (instrumented, branches) = instrument(source);
    let (combinations, total, sampled) = combinations(&branches.chains);
    let counts: &[usize] = if branches.loops == 0 {
        &[1]
    } else {
        &LOOP_COUNTS
    };
    let env = environment();
    let template = match env.template_from_str(&instrumented) {
        Ok(template) => template,
        Err(e) => {
            return Rendering {
                branches,
                combinations: total,
                sampled,
                variants: vec![Variant {
                    label: "compile".to_owned(),
                    text: Err(e.to_string()),
                }],
            };
        }
    };
    let undeclared = template.undeclared_variables(false);
    let mut stubs: BTreeMap<String, Value> = BTreeMap::new();
    for name in undeclared {
        if name.starts_with(INTERNAL) || env.globals().any(|(global, _)| global == name) {
            continue;
        }
        let value = dbt_stub(&name).unwrap_or_else(|| Stub::value(&name));
        stubs.insert(name, value);
    }
    let mut variants = Vec::with_capacity(combinations.len() * counts.len());
    for choice in &combinations {
        for &count in counts {
            let mut context = stubs.clone();
            let chosen = Arc::new(choice.clone());
            context.insert(
                BRANCH.to_owned(),
                Value::from_function(move |chain: usize, index: usize| {
                    chosen.get(chain).is_some_and(|&c| c == index)
                }),
            );
            context.insert(
                LOOP.to_owned(),
                Value::from_function(move |iterable: Value| take(&iterable, count)),
            );
            let label = label(choice, branches.loops > 0, count);
            let text = template
                .render(Value::from(context))
                .map_err(|e| describe(&e));
            variants.push(Variant { label, text });
        }
    }
    Rendering {
        branches,
        combinations: total,
        sampled,
        variants,
    }
}

/// The distinct pairs of renders of `input` and of `output`, formatted from
/// it, that both render, with the label of the first variant to give each:
/// what the oracle judges for a template. None where the two have different
/// branches, which only a formatter bug can cause and [`compare_sql`] over
/// the variants shows.
#[must_use]
pub fn rendered_pairs(input: &str, output: &str) -> Vec<(String, String, String)> {
    let (before, after) = (render(input), render(output));
    if before.branches != after.branches {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (a, b) in before.variants.into_iter().zip(after.variants) {
        if let (Ok(x), Ok(y)) = (a.text, b.text)
            && seen.insert((x.clone(), y.clone()))
        {
            out.push((a.label, x, y));
        }
    }
    out
}

/// The prefix of the names this module adds to a template.
const INTERNAL: &str = "__chairlift_";
/// The function that decides a branch: `__chairlift_branch(chain, index)`.
const BRANCH: &str = "__chairlift_branch";
/// The function that cuts a loop's iterable: `__chairlift_loop(iterable)`.
const LOOP: &str = "__chairlift_loop";

fn label(choice: &[usize], loops: bool, count: usize) -> String {
    let mut out = String::from("branches ");
    if choice.is_empty() {
        out.push('-');
    }
    for (k, c) in choice.iter().enumerate() {
        if k > 0 {
            out.push('.');
        }
        out.push_str(&c.to_string());
    }
    if loops {
        out.push_str(", loops ");
        out.push_str(&count.to_string());
    }
    out
}

/// An error with the errors that caused it, on one line.
fn describe(e: &RenderError) -> String {
    let mut out = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        out.push_str(": ");
        out.push_str(&s.to_string());
        source = s.source();
    }
    out.replace('\n', " ")
}

/// chum's environment, with the stub filters.
fn environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_auto_escape_callback(|_| AutoEscape::None);
    if let Ok(syntax) = SyntaxConfig::builder()
        .trim_blocks(false)
        .lstrip_blocks(false)
        .keep_trailing_newline(true)
        .build()
    {
        env.set_syntax(syntax);
    }
    env.set_fuel(Some(FUEL));
    env.add_filter("sql_string", |v: Value| quote(&v.to_string(), '\''));
    env.add_filter("sql_ident", |v: Value| quote(&v.to_string(), '`'));
    env.add_filter("sql_date", |_: Value| "toDate('2026-11-10')".to_owned());
    env.add_filter("sql_datetime", |_: Value| {
        "toDateTime('2026-11-10 08:00:00', 'UTC')".to_owned()
    });
    env.add_filter("sql_datetime64", |_: Value, precision: u32| {
        format!("toDateTime64('2026-11-10 08:00:00', {precision}, 'UTC')")
    });
    // What ClickHouse's test runner renders its templates with: Jinja2's
    // `random` filter, here its first item so that renders repeat, and
    // `itertools.product`.
    env.add_filter("random", |v: Value| -> Result<Value, RenderError> {
        Ok(v.try_iter()?.next().unwrap_or_default())
    });
    env.add_function(
        "product",
        |seqs: Rest<Value>| -> Result<Value, RenderError> {
            let mut out: Vec<Vec<Value>> = vec![Vec::new()];
            for seq in seqs.iter() {
                let items: Vec<Value> = seq.try_iter()?.collect();
                out = out
                    .into_iter()
                    .flat_map(|prefix| {
                        items.iter().map(move |item| {
                            let mut next = prefix.clone();
                            next.push(item.clone());
                            next
                        })
                    })
                    .collect();
            }
            Ok(Value::from(
                out.into_iter().map(Value::from).collect::<Vec<_>>(),
            ))
        },
    );
    env.set_unknown_method_callback(python_method);
    env
}

/// The Python methods of maps, strings and lists that templates written for
/// Jinja2 call.
fn python_method(
    _: &mut State<'_, '_>,
    value: &Value,
    method: &str,
    args: &[Value],
) -> Result<Value, RenderError> {
    let unknown = || RenderError::from(minijinja::ErrorKind::UnknownMethod);
    if let Some(text) = value.as_str() {
        let arg = |n: usize| args.get(n).and_then(Value::as_str).ok_or_else(unknown);
        return Ok(match method {
            "startswith" => Value::from(text.starts_with(arg(0)?)),
            "endswith" => Value::from(text.ends_with(arg(0)?)),
            "upper" => Value::from(text.to_uppercase()),
            "lower" => Value::from(text.to_lowercase()),
            "strip" => Value::from(text.trim()),
            "replace" => Value::from(text.replace(arg(0)?, arg(1)?)),
            "split" => Value::from(match args.first().and_then(Value::as_str) {
                Some(sep) => text.split(sep).map(Value::from).collect::<Vec<_>>(),
                None => text.split_whitespace().map(Value::from).collect(),
            }),
            _ => return Err(unknown()),
        });
    }
    if value.kind() == minijinja::value::ValueKind::Map {
        let pairs = value.try_iter()?.map(|k| {
            let v = value.get_item(&k).unwrap_or_default();
            (k, v)
        });
        return Ok(match method {
            "items" => Value::from(
                pairs
                    .map(|(k, v)| Value::from(vec![k, v]))
                    .collect::<Vec<_>>(),
            ),
            "keys" => Value::from(pairs.map(|(k, _)| k).collect::<Vec<_>>()),
            "values" => Value::from(pairs.map(|(_, v)| v).collect::<Vec<_>>()),
            _ => return Err(unknown()),
        });
    }
    Err(unknown())
}

/// `text` in `quote` characters, with the quote character and `\` escaped by
/// a backslash.
fn quote(text: &str, quote: char) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for c in text.chars() {
        if c == quote || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push(quote);
    out
}

/// The stub for one of dbt's functions, if `name` is one whose rendering a
/// model depends on.
fn dbt_stub(name: &str) -> Option<Value> {
    Some(match name {
        "config" | "print" | "log" => {
            Value::from_function(|_: Rest<Value>, _: Kwargs| String::new())
        }
        "ref" => Value::from_function(|args: Rest<Value>, _: Kwargs| {
            args.last()
                .map_or_else(|| "ref".to_owned(), |a| identifier(&a.to_string()))
        }),
        "source" => Value::from_function(|args: Rest<Value>, _: Kwargs| {
            args.iter()
                .map(|a| identifier(&a.to_string()))
                .collect::<Vec<_>>()
                .join(".")
        }),
        "is_incremental" => Value::from_function(|| false),
        "var" | "env_var" => Value::from_function(|args: Rest<Value>, _: Kwargs| match &args[..] {
            [_, default, ..] => default.clone(),
            [name] => Stub::value(&name.to_string()),
            [] => Stub::value("var"),
        }),
        _ => return None,
    })
}

/// `text` as a bare identifier: characters other than ASCII letters, digits
/// and `_` become `_`, and a leading digit gets a `_` before it.
fn identifier(text: &str) -> String {
    let mut out: String = text
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// A value for a name the template uses without defining it.
#[derive(Debug)]
struct Stub {
    name: String,
}

impl Stub {
    fn value(name: &str) -> Value {
        Value::from_object(Stub {
            name: identifier(name),
        })
    }
}

impl Object for Stub {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Plain
    }

    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        Some(Stub::value(&format!("{}_{key}", self.name)))
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::NonEnumerable
    }

    fn is_true(self: &Arc<Self>) -> bool {
        true
    }

    fn call(self: &Arc<Self>, _: &mut State<'_, '_>, _: &[Value]) -> Result<Value, RenderError> {
        Ok(Value::from_dyn_object(self.clone()))
    }

    fn call_method(
        self: &Arc<Self>,
        _: &mut State<'_, '_>,
        method: &str,
        _: &[Value],
    ) -> Result<Value, RenderError> {
        Ok(Stub::value(&format!("{}_{method}", self.name)))
    }

    fn render(self: &Arc<Self>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// The first `count` items of `iterable`, repeating them where it has fewer;
/// a stub gives `count` stubs.
fn take(iterable: &Value, count: usize) -> Result<Value, RenderError> {
    if let Some(stub) = iterable.downcast_object_ref::<Stub>() {
        return Ok(Value::from(
            (0..count)
                .map(|i| Stub::value(&format!("{}{i}", stub.name)))
                .collect::<Vec<_>>(),
        ));
    }
    let items: Vec<Value> = iterable.try_iter()?.collect();
    if items.is_empty() {
        return Ok(Value::from(items));
    }
    Ok(Value::from(
        items
            .iter()
            .cycle()
            .take(count)
            .cloned()
            .collect::<Vec<_>>(),
    ))
}

/// The open blocks while instrumenting.
enum Open {
    /// An `{% if %}` of this chain.
    If(usize),
    /// Any other block.
    Other,
}

/// `source` with the condition of every `{% if %}` and `{% elif %}` replaced
/// by a call that decides it, and the iterable of every `{% for %}` loop it
/// can rewrite by a call that cuts it, with the branches and loops it found.
/// Every other byte stays, markers included, so the render keeps the
/// template's whitespace.
fn instrument(source: &str) -> (String, Branches) {
    let mut out = String::with_capacity(source.len() + 64);
    let mut branches = Branches::default();
    let mut open: Vec<Open> = Vec::new();
    let mut pos = 0;
    for tag in jinja::tags(source) {
        if tag.kind != TagKind::Block || !tag.closed {
            continue;
        }
        let text = &source[tag.range.clone()];
        let (before, after) = tag.whitespace_control(source);
        let inner_start = 2 + usize::from(before != WhitespaceControl::Default);
        let inner_end = text.len() - 2 - usize::from(after != WhitespaceControl::Default);
        let Some(inner) = text.get(inner_start..inner_end) else {
            continue;
        };
        let inner = inner.trim();
        let name_len = inner
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(inner.len());
        let (name, rest) = inner.split_at(name_len);
        let replacement = match name {
            "if" => {
                let chain = branches.chains.len();
                branches.chains.push(2);
                open.push(Open::If(chain));
                Some(format!("if {BRANCH}({chain}, 0)"))
            }
            "elif" => match open.last() {
                Some(&Open::If(chain)) => {
                    let index = branches.chains[chain] - 1;
                    branches.chains[chain] += 1;
                    Some(format!("elif {BRANCH}({chain}, {index})"))
                }
                _ => None,
            },
            "for" => {
                open.push(Open::Other);
                let rewritten = loop_iterable(rest);
                if rewritten.is_some() {
                    branches.loops += 1;
                }
                rewritten
            }
            "macro" | "call" | "filter" | "block" | "with" | "autoescape" | "trans" => {
                open.push(Open::Other);
                None
            }
            "set" if !rest.contains('=') => {
                open.push(Open::Other);
                None
            }
            _ if name.starts_with("end") => {
                open.pop();
                None
            }
            _ => None,
        };
        if let Some(replacement) = replacement {
            out.push_str(&source[pos..tag.range.start]);
            out.push_str("{%");
            out.push_str(marker(before));
            out.push(' ');
            out.push_str(&replacement);
            out.push(' ');
            out.push_str(marker(after));
            out.push_str("%}");
            pos = tag.range.end;
        }
    }
    out.push_str(&source[pos..]);
    (out, branches)
}

fn marker(control: WhitespaceControl) -> &'static str {
    match control {
        WhitespaceControl::Default => "",
        WhitespaceControl::Strip => "-",
        WhitespaceControl::Preserve => "+",
    }
}

/// The body of a `{% for %}` tag after `for`, `x in xs`, with the iterable
/// cut by the loop function: `for x in __chairlift_loop(xs)`; `None` for a
/// loop with a filter (`if`) or `recursive`, which runs as written.
fn loop_iterable(rest: &str) -> Option<String> {
    let words = top_level_words(rest);
    let &(in_start, in_end) = words.iter().find(|&&(s, e)| &rest[s..e] == "in")?;
    if words
        .iter()
        .any(|&(s, e)| s > in_start && matches!(&rest[s..e], "if" | "recursive"))
    {
        return None;
    }
    let target = rest[..in_start].trim();
    let iterable = rest[in_end..].trim();
    if target.is_empty() || iterable.is_empty() {
        return None;
    }
    Some(format!("for {target} in {LOOP}({iterable})"))
}

/// The byte ranges of the words of a Jinja expression outside brackets and
/// strings.
fn top_level_words(text: &str) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let mut words = Vec::new();
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            q @ (b'\'' | b'"') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'(' | b'[' | b'{' => {
                depth += 1;
                i += 1;
            }
            b')' | b']' | b'}' => {
                depth -= 1;
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                if depth == 0 {
                    words.push((start, i));
                }
            }
            _ => i += 1,
        }
    }
    words
}

/// The combinations of branches to render: every one if there are at most
/// [`MAX_COMBINATIONS`], otherwise that many, each chain's first branch
/// everywhere, its last everywhere, and a fixed sample; with how many there
/// are and whether this is a sample.
fn combinations(chains: &[usize]) -> (Vec<Vec<usize>>, u64, bool) {
    let total = chains
        .iter()
        .try_fold(1u64, |a, &n| a.checked_mul(n as u64))
        .unwrap_or(u64::MAX);
    if total <= MAX_COMBINATIONS as u64 {
        let mut out = Vec::new();
        for mut m in 0..total {
            let mut choice = Vec::with_capacity(chains.len());
            for &n in chains {
                choice.push(usize::try_from(m % n as u64).unwrap_or(0));
                m /= n as u64;
            }
            out.push(choice);
        }
        return (out, total, false);
    }
    let mut out = vec![
        vec![0; chains.len()],
        chains.iter().map(|n| n - 1).collect(),
    ];
    let mut state = 0x2026_1111_u64;
    while out.len() < MAX_COMBINATIONS {
        let choice = chains
            .iter()
            .map(|&n| {
                state = splitmix(state);
                usize::try_from(state % n as u64).unwrap_or(0)
            })
            .collect();
        out.push(choice);
    }
    (out, total, true)
}

fn splitmix(state: u64) -> u64 {
    let mut z = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Whether two rendered texts hold the same SQL: the same tokens apart from
/// whitespace, where a bare word may differ only by being in its canonical
/// case in `output` (a keyword upper case, `true`, `false`, `inf`,
/// `infinity` and `nan` lower case), a line comment only in the whitespace
/// at its end, and the literal left of `::`, whose text ClickHouse reads,
/// not at all, whitespace included. `INSERT` data is one token (see
/// [`split`](crate::split)).
///
/// # Errors
///
/// The first difference, described.
pub fn compare_sql(input: &str, output: &str) -> Result<(), String> {
    let a = sql_tokens(input);
    let b = sql_tokens(output);
    for (n, (x, y)) in a.iter().zip(&b).enumerate() {
        let same = x.0 == y.0
            && (x.1 == y.1
                || (x.0 == TokenKind::BareWord && canonical_case(&x.1).is_some_and(|c| c == y.1)));
        if !same {
            return Err(format!(
                "token {} is {:?} {:?} in the input's render and {:?} {:?} in the output's",
                n + 1,
                x.0,
                excerpt(&x.1),
                y.0,
                excerpt(&y.1)
            ));
        }
    }
    if a.len() == b.len() {
        Ok(())
    } else {
        Err(format!(
            "the input's render has {} tokens and the output's {}",
            a.len(),
            b.len()
        ))
    }
}

fn excerpt(text: &str) -> String {
    match text.char_indices().nth(60) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

fn canonical_case(word: &str) -> Option<String> {
    let lower = word.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "true" | "false" | "inf" | "infinity" | "nan"
    ) {
        Some(lower)
    } else if Keyword::from_word(word).is_some() {
        Some(word.to_ascii_uppercase())
    } else {
        None
    }
}

/// The SQL of `text` as [`compare_sql`] compares it: its tokens without
/// whitespace, line comments without the whitespace at their end, and after
/// each `::` that follows a bracket, the source text of the bracketed
/// literal before it.
fn sql_tokens(text: &str) -> Vec<(TokenKind, String)> {
    let split = crate::split::split(text, Mode::Sql);
    let tokens = split.tokens();
    let solid: Vec<usize> = (0..tokens.len())
        .filter(|&i| tokens[i].kind() != TokenKind::Whitespace)
        .collect();
    let mut out = Vec::with_capacity(solid.len());
    for (n, &i) in solid.iter().enumerate() {
        let t = tokens[i];
        let s = t.text(text);
        let s = if t.kind() == TokenKind::LineComment {
            crate::verify::line_comment_text(s)
        } else {
            s
        };
        out.push((t.kind(), s.to_owned()));
        if t.kind() == TokenKind::DoubleColon
            && let Some(literal) = literal_before(text, tokens, &solid, n)
        {
            out.push((TokenKind::DoubleColon, format!("literal {literal}")));
        }
    }
    out
}

/// The source text of the bracketed literal that ends just before the `n`th
/// solid token, a `::`: an array or tuple of numbers, strings and literal
/// words, not the arguments of a call.
fn literal_before<'t>(
    text: &'t str,
    tokens: &[crate::lexer::Token],
    solid: &[usize],
    n: usize,
) -> Option<&'t str> {
    let last = *solid.get(n.checked_sub(1)?)?;
    let close = tokens[last].kind();
    let open = match close {
        TokenKind::ClosingSquareBracket => TokenKind::OpeningSquareBracket,
        TokenKind::ClosingRoundBracket => TokenKind::OpeningRoundBracket,
        _ => return None,
    };
    let mut depth = 0usize;
    for m in (0..n).rev() {
        let token = tokens[solid[m]];
        let kind = token.kind();
        if kind == close {
            depth += 1;
        } else if kind == open {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                let call = m.checked_sub(1).is_some_and(|k| {
                    let before = tokens[solid[k]];
                    match before.kind() {
                        TokenKind::BareWord => !operand_before(before.text(text)),
                        TokenKind::QuotedIdentifier
                        | TokenKind::ClosingRoundBracket
                        | TokenKind::ClosingSquareBracket => true,
                        _ => false,
                    }
                });
                return (!call).then(|| text.get(token.start()..tokens[last].end()))?;
            }
        } else if !literal_part(kind, token.text(text)) {
            return None;
        }
    }
    None
}

/// Whether `word` is a keyword after which brackets start an operand, not
/// the arguments of a call or an index: `SELECT [1, 2]::String`.
fn operand_before(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "select"
            | "where"
            | "prewhere"
            | "having"
            | "and"
            | "or"
            | "not"
            | "in"
            | "is"
            | "like"
            | "between"
            | "when"
            | "then"
            | "else"
            | "by"
            | "on"
            | "distinct"
            | "values"
    )
}

/// Whether a token can be part of an array or tuple literal.
fn literal_part(kind: TokenKind, text: &str) -> bool {
    match kind {
        TokenKind::Number
        | TokenKind::StringLiteral
        | TokenKind::Comma
        | TokenKind::Minus
        | TokenKind::Plus
        | TokenKind::OpeningRoundBracket
        | TokenKind::ClosingRoundBracket
        | TokenKind::OpeningSquareBracket
        | TokenKind::ClosingSquareBracket => true,
        TokenKind::BareWord => matches!(
            text.to_ascii_lowercase().as_str(),
            "null" | "true" | "false" | "inf" | "infinity" | "nan"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
