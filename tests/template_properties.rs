//! Generated templates and queries through `chairlift::format`: the output
//! verifies (`format` checks that), formatting it again changes nothing, and
//! every render of a template, with each branch taken or left out and each
//! loop run zero, one or two times, has the same SQL tokens before and after
//! formatting. The verifier itself checks only the render with every branch
//! taken.
//!
//! `CHAIRLIFT_TEMPLATE_CASES` sets how many texts of each kind are tried
//! (`mise run test:templates` runs many), and `CHAIRLIFT_TEMPLATE_SEED`
//! another sequence of them.

use chairlift::Error;
use chairlift::Options;
use chairlift::lexer::Mode;
use chairlift::lexer::TokenKind;
use chairlift::lexer::tokenize;

/// A small deterministic random number generator (SplitMix64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap_or(0)
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

fn cases() -> usize {
    std::env::var("CHAIRLIFT_TEMPLATE_CASES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(300)
}

fn rng(seed: u64) -> Rng {
    let offset = std::env::var("CHAIRLIFT_TEMPLATE_SEED")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    Rng(seed ^ offset)
}

/// Whitespace and the comments that can stand in it.
const GAPS: &[&str] = &[
    " ",
    " ",
    "\n",
    "\n    ",
    "\n\n",
    " -- c\n",
    "\n-- own\n",
    " /* b */ ",
    "\n/* b */ ",
    " {# j #} ",
    "\n{# j #}\n",
];

const ATOMS: &[&str] = &[
    "a",
    "b.c",
    "1",
    "'s'",
    "f(a, 2)",
    "{{ x }}",
    "[1,2]::String",
    "[{{ x }},2]::String",
    "x + y * 2",
    "(a OR b)",
    "case when a then 1 else 2 end",
    "'multi\nline'",
    "(select 1)",
    "-1",
];

/// One query, with tags around its items and clauses.
fn query(r: &mut Rng, out: &mut String) {
    let gap = |r: &mut Rng| r.pick(GAPS);
    out.push_str("select");
    let items = 1 + r.below(4);
    for k in 0..items {
        if k > 0 {
            out.push_str(r.pick(&[",", " ,", ","]));
        }
        out.push_str(gap(r));
        match r.below(8) {
            0 => {
                out.push_str(r.pick(&["{% if x %}", "{%- if x %}", "{% if x -%}"]));
                out.push_str(gap(r));
                out.push_str(r.pick(ATOMS));
                out.push_str(", ");
                out.push_str(r.pick(&["{% endif %}", "{%- endif %}", "{% endif -%}"]));
                out.push_str(gap(r));
                out.push_str(r.pick(ATOMS));
            }
            1 => {
                out.push_str("{% for i in xs %} c{{ i }}, {% endfor %}");
                out.push_str(gap(r));
                out.push_str(r.pick(ATOMS));
            }
            _ => out.push_str(r.pick(ATOMS)),
        }
    }
    out.push_str(gap(r));
    out.push_str("from t");
    if r.below(2) == 0 {
        out.push_str(gap(r));
        out.push_str("where ");
        out.push_str(r.pick(ATOMS));
        out.push_str(gap(r));
        if r.below(2) == 0 {
            out.push_str(r.pick(&["{% if y %}", "{%- if y %}"]));
            out.push_str(gap(r));
            out.push_str("and b = 1");
            out.push_str(gap(r));
            out.push_str("{% endif %}");
        } else {
            out.push_str("and c = 2");
        }
    }
    if r.below(3) == 0 {
        out.push_str(gap(r));
        out.push_str(r.pick(&[
            "group by a, b",
            "order by a desc",
            "limit 3 by a",
            "format CSV",
        ]));
    }
}

/// One `CREATE TABLE` or materialized view, with tags among its columns and
/// clauses.
fn create(r: &mut Rng, out: &mut String) {
    let gap = |r: &mut Rng| r.pick(GAPS);
    let view = r.below(3) == 0;
    out.push_str(if view {
        "create materialized view v to t"
    } else {
        "create table t"
    });
    out.push_str(" (");
    let columns = 1 + r.below(4);
    for k in 0..columns {
        if k > 0 {
            out.push(',');
        }
        out.push_str(gap(r));
        match r.below(6) {
            0 => {
                out.push_str("{% if x %}");
                out.push_str(gap(r));
                out.push_str("b String,");
                out.push_str(gap(r));
                out.push_str(r.pick(&["{% endif %}", "{%- endif %}"]));
                out.push_str(gap(r));
                out.push_str("c Int32");
            }
            1 => out.push_str("d Int64 default {{ x }}"),
            2 => out.push_str("e Array(String) codec(ZSTD(1))"),
            _ => out.push_str(r.pick(&["a UInt8", "f Nullable(String)", "g DateTime64(3)"])),
        }
    }
    out.push_str(gap(r));
    out.push(')');
    out.push_str(gap(r));
    out.push_str("engine = MergeTree");
    out.push_str(gap(r));
    out.push_str(r.pick(&["order by (a, b)", "order by a", "partition by toYYYYMM(g)"]));
    if r.below(2) == 0 {
        out.push_str(gap(r));
        out.push_str(r.pick(&[
            "{% if y %} ttl g + interval 1 day {% endif %}",
            "settings index_granularity = 8192",
        ]));
    }
    if view {
        out.push_str(gap(r));
        out.push_str("as");
        out.push_str(gap(r));
        query(r, out);
    }
}

fn template(r: &mut Rng) -> String {
    let mut out = String::new();
    let statements = 1 + r.below(3);
    for k in 0..statements {
        if k > 0 {
            out.push_str(r.pick(&[";\n", ";\n\n", "; ", ";\n{% if z %}\n", ";\n{# s #}\n"]));
        }
        if r.below(4) == 0 {
            create(r, &mut out);
        } else {
            query(r, &mut out);
        }
    }
    if out.contains("{% if z %}") {
        out.push_str("\n{% endif %}");
    }
    if r.below(2) == 0 {
        out.push(';');
    }
    out
}

/// The SQL tokens of a text, without whitespace, with bare words in one case.
fn sql_tokens(text: &str) -> Vec<(TokenKind, String)> {
    tokenize(text, Mode::Sql)
        .iter()
        .filter(|t| t.kind() != TokenKind::Whitespace)
        .map(|t| {
            let text = t.text(text);
            let text = match t.kind() {
                TokenKind::LineComment => text.trim_end().to_owned(),
                TokenKind::BareWord => text.to_ascii_lowercase(),
                _ => text.to_owned(),
            };
            (t.kind(), text)
        })
        .collect()
}

fn renders(template: &str) -> Vec<Option<String>> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Lenient);
    let mut out = Vec::new();
    for x in [false, true] {
        for y in [false, true] {
            for xs in [0, 1, 2] {
                let ctx = minijinja::Value::from_iter([
                    ("x", minijinja::Value::from(x)),
                    ("y", minijinja::Value::from(y)),
                    ("z", minijinja::Value::from(x)),
                    ("xs", minijinja::Value::from((1..=xs).collect::<Vec<i64>>())),
                ]);
                out.push(env.render_str(template, ctx).ok());
            }
        }
    }
    out
}

/// Formats `source`; `None` where it is left as it is by design.
fn format(source: &str, jinja: bool) -> Option<String> {
    let options = Options::default().with_jinja(jinja).with_line_width(40);
    match chairlift::format(source, &options) {
        Ok(out) => Some(out),
        Err(Error::Lex(_) | Error::Template(_)) => None,
        Err(e) => panic!("{e}\n--- input:\n{source}"),
    }
}

#[test]
fn every_render_of_a_formatted_template_keeps_its_tokens() {
    let mut r = rng(2026);
    for _ in 0..cases() {
        let source = template(&mut r);
        let Some(once) = format(&source, true) else {
            continue;
        };
        let twice = format(&once, true);
        assert_eq!(twice.as_ref(), Some(&once), "not idempotent:\n{source}");
        for (before, after) in renders(&source).into_iter().zip(renders(&once)) {
            if let (Some(before), Some(after)) = (before, after) {
                assert_eq!(
                    sql_tokens(&before),
                    sql_tokens(&after),
                    "a render differs\n--- input:\n{source}\n--- output:\n{once}"
                );
            }
        }
    }
}

#[test]
fn generated_queries_format_idempotently() {
    let mut r = rng(7);
    for _ in 0..cases() {
        let mut source = String::new();
        query(&mut r, &mut source);
        let source = source.replace("{%", "/*").replace("%}", "*/");
        let source = source.replace("{{ x }}", "x").replace("{# j #}", "/* j */");
        let source = source.replace("{{ i }}", "i");
        let Some(once) = format(&source, false) else {
            continue;
        };
        let twice = format(&once, false);
        assert_eq!(twice.as_ref(), Some(&once), "not idempotent:\n{source}");
    }
}
