//! Generated templates and queries through `chairlift::format`: the output
//! verifies (`format` checks that), formatting it again changes nothing, and
//! every render of a template, with each branch taken or left out and each
//! loop run zero, one or two times, has the same SQL before and after
//! formatting: the same tokens, and the same bytes where ClickHouse reads
//! source text (strings, and the literal left of `::`). The verifier itself
//! checks only the render with every branch taken.
//!
//! `CHAIRLIFT_TEMPLATE_CASES` sets how many texts of each kind are tried
//! (`mise run test:templates` runs many), and `CHAIRLIFT_TEMPLATE_SEED`
//! another sequence of them.

use chairlift::Error;
use chairlift::Options;
use chairlift::lexer::Mode;
use chairlift::lexer::TokenKind;

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
    " -- c {{ v }}\n",
];

/// Comments with a `-` marker that strips the line break ending them, which
/// leave most templates as they are: rarer than [`GAPS`].
const MARKED_GAPS: &[&str] = &[" -- c {{ v -}}\n", "\n-- e {{- v -}}\n"];

/// Whitespace or a comment, now and then one whose line break a `-` marker
/// strips.
fn gap(r: &mut Rng) -> &'static str {
    if r.below(100) == 0 {
        r.pick(MARKED_GAPS)
    } else {
        r.pick(GAPS)
    }
}

const ATOMS: &[&str] = &[
    "a",
    "b.c",
    "1",
    "'s'",
    "f(a, 2)",
    "{{ x }}",
    "[1,2]::String",
    "[{{ x }},2]::String",
    "(1, {{ v }})::Tuple(UInt8, UInt8)",
    "x + y * 2",
    "(a OR b)",
    "case when a then 1 else 2 end",
    "'multi\nline'",
    "'{{ v }},{{ v }}'",
    "(select 1)",
    "-1",
];

/// A block tag, with `-` markers on either side now and then.
fn tag(r: &mut Rng, body: &str) -> String {
    let left = if r.below(6) == 0 { "{%-" } else { "{%" };
    let right = if r.below(6) == 0 { "-%}" } else { "%}" };
    format!("{left} {body} {right}")
}

/// One query, with tags around its items and clauses.
fn query(r: &mut Rng, out: &mut String) {
    out.push_str("select");
    let items = 1 + r.below(4);
    for k in 0..items {
        if k > 0 {
            out.push_str(r.pick(&[",", " ,", ","]));
        }
        out.push_str(gap(r));
        match r.below(9) {
            0 => {
                out.push_str(r.pick(&["{% if x %}", "{%- if x %}", "{% if x -%}"]));
                out.push_str(gap(r));
                out.push_str(r.pick(ATOMS));
                out.push_str(", ");
                if r.below(3) == 0 {
                    out.push_str(&tag(r, "else"));
                    out.push_str(gap(r));
                    out.push_str(r.pick(ATOMS));
                    out.push_str(", ");
                }
                out.push_str(r.pick(&["{% endif %}", "{%- endif %}", "{% endif -%}"]));
                out.push_str(gap(r));
                out.push_str(r.pick(ATOMS));
            }
            1 => {
                out.push_str("{% for i in xs %} c{{ i }}, {% endfor %}");
                out.push_str(gap(r));
                out.push_str(r.pick(ATOMS));
            }
            2 => {
                out.push_str(&tag(r, "for i in xs"));
                out.push_str(gap(r));
                out.push_str("c{{ i }}");
                out.push_str(&tag(r, "if not loop.last"));
                out.push(',');
                out.push_str(&tag(r, "endif"));
                out.push_str(gap(r));
                out.push_str(&tag(r, "endfor"));
            }
            _ => out.push_str(r.pick(ATOMS)),
        }
    }
    out.push_str(gap(r));
    out.push_str(r.pick(&["from t", "from {{ db }}.t", "from t_{{ s }}"]));
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
            if r.below(3) == 0 {
                out.push_str(&tag(r, "else"));
                out.push_str(gap(r));
                out.push_str("and b = 2");
                out.push_str(gap(r));
            }
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
            "settings max_threads = {{ g }}",
        ]));
    }
}

/// A column or other element of a `CREATE TABLE`.
fn column(r: &mut Rng, out: &mut String) {
    out.push_str(r.pick(&[
        "a UInt8",
        "f Nullable(String)",
        "g DateTime64(3)",
        "d Int64 default {{ x }}",
        "e Array(String) codec(ZSTD(1))",
        "h String default {% raw %}'{{ x }}'{% endraw %}",
        "k Array(UInt8) default [{{ v }},2]::Array(UInt8)",
        "l String default '[{{ v }}, 2]'",
        "m UInt8 default {{- v }}",
        "n String comment 'multi\nline'",
        "{{ v }}_c UInt8",
    ]));
}

/// One `CREATE TABLE` or materialized view, with tags among its columns and
/// clauses.
fn create(r: &mut Rng, out: &mut String) {
    let view = r.below(3) == 0;
    out.push_str(if view {
        "create materialized view v to t"
    } else {
        r.pick(&[
            "create table t",
            "create table {{ db }}.t",
            "create table t_{{ s }}",
        ])
    });
    if r.below(4) == 0 {
        out.push_str(r.pick(&[" on cluster c", " on cluster {{ c }}"]));
    }
    out.push_str(" (");
    let columns = 1 + r.below(4);
    for k in 0..columns {
        if k > 0 {
            out.push(',');
        }
        out.push_str(gap(r));
        match r.below(7) {
            0 => {
                out.push_str(&tag(r, "if x"));
                out.push_str(gap(r));
                out.push_str("b String,");
                out.push_str(gap(r));
                if r.below(2) == 0 {
                    out.push_str(&tag(r, "else"));
                    out.push_str(gap(r));
                    out.push_str("b UInt8,");
                    out.push_str(gap(r));
                }
                out.push_str(&tag(r, "endif"));
                out.push_str(gap(r));
                out.push_str("c Int32");
            }
            1 => {
                out.push_str(&tag(r, "for i in xs"));
                out.push_str(gap(r));
                out.push_str("c{{ i }} UInt8");
                out.push_str(&tag(r, "if not loop.last"));
                out.push(',');
                out.push_str(&tag(r, "endif"));
                out.push_str(gap(r));
                out.push_str(&tag(r, "endfor"));
                out.push_str(",\n");
                column(r, out);
            }
            _ => column(r, out),
        }
    }
    out.push_str(gap(r));
    out.push(')');
    out.push_str(gap(r));
    out.push_str(r.pick(&["engine = MergeTree", "engine = {{ engine }}"]));
    out.push_str(gap(r));
    out.push_str(r.pick(&["order by (a, b)", "order by a", "partition by toYYYYMM(g)"]));
    if r.below(2) == 0 {
        out.push_str(gap(r));
        out.push_str(r.pick(&[
            "{% if y %} ttl g + interval 1 day {% endif %}",
            "{% if y %} ttl g + interval 1 day {% else %} ttl g + interval 2 day {% endif %}",
            "settings index_granularity = 8192",
            "settings index_granularity = {{ g }}",
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

/// The SQL of a rendered text as ClickHouse reads it: its tokens without
/// whitespace, bare words in one case and `INSERT` data as one token, and
/// the source text of each literal in brackets left of `::`, which
/// ClickHouse reads as written.
fn sql(text: &str) -> Vec<(TokenKind, String)> {
    let split = chairlift::split::split(text, Mode::Sql);
    let tokens = split.tokens();
    let solid: Vec<usize> = (0..tokens.len())
        .filter(|&i| tokens[i].kind() != TokenKind::Whitespace)
        .collect();
    let mut out: Vec<(TokenKind, String)> = solid
        .iter()
        .map(|&i| {
            let t = tokens[i];
            let s = t.text(text);
            let s = match t.kind() {
                TokenKind::LineComment => s.trim_end().to_owned(),
                TokenKind::BareWord => s.to_ascii_lowercase(),
                _ => s.to_owned(),
            };
            (t.kind(), s)
        })
        .collect();
    for (n, &i) in solid.iter().enumerate().skip(1) {
        if tokens[i].kind() != TokenKind::DoubleColon {
            continue;
        }
        let close = tokens[solid[n - 1]].kind();
        let open = match close {
            TokenKind::ClosingSquareBracket => TokenKind::OpeningSquareBracket,
            TokenKind::ClosingRoundBracket => TokenKind::OpeningRoundBracket,
            _ => continue,
        };
        let mut depth = 0;
        let mut m = n - 1;
        loop {
            let kind = tokens[solid[m]].kind();
            if kind == close {
                depth += 1;
            } else if kind == open {
                depth -= 1;
                if depth == 0 || m == 0 {
                    break;
                }
            }
            if m == 0 {
                break;
            }
            m -= 1;
        }
        let literal = &text[tokens[solid[m]].start()..tokens[solid[n - 1]].end()];
        out.push((TokenKind::DoubleColon, format!("literal {literal}")));
    }
    out
}

/// Every render of `template` in the contexts the tests use, `None` where
/// the renderer fails.
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
                    ("v", minijinja::Value::from("7")),
                    ("s", minijinja::Value::from("sfx")),
                    ("db", minijinja::Value::from("db")),
                    ("engine", minijinja::Value::from("Memory")),
                    ("g", minijinja::Value::from("8192")),
                    ("c", minijinja::Value::from("cc")),
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

/// The share of generated templates that may be left as they are: more,
/// and the test checks too little to mean anything.
const MAX_LEFT_ALONE: f64 = 0.4;

#[test]
fn every_render_of_a_formatted_template_keeps_its_sql() {
    let mut r = rng(2026);
    let mut left_alone = 0_u32;
    let n = cases();
    for _ in 0..n {
        let source = template(&mut r);
        let Some(once) = format(&source, true) else {
            left_alone += 1;
            continue;
        };
        let twice = format(&once, true);
        assert_eq!(twice.as_ref(), Some(&once), "not idempotent:\n{source}");
        for (before, after) in renders(&source).into_iter().zip(renders(&once)) {
            match (before, after) {
                (Some(before), Some(after)) => assert_eq!(
                    sql(&before),
                    sql(&after),
                    "a render differs\n--- input:\n{source}\n--- output:\n{once}"
                ),
                (None, None) => {}
                (before, _) => panic!(
                    "only the {} renders\n--- input:\n{source}\n--- output:\n{once}",
                    if before.is_some() { "input" } else { "output" }
                ),
            }
        }
    }
    eprintln!("{left_alone} of {n} templates were left as they are");
    let share = f64::from(left_alone) / f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    assert!(
        share <= MAX_LEFT_ALONE,
        "{left_alone} of {n} templates were left as they are"
    );
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
