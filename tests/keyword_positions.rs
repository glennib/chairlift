//! Every ClickHouse keyword word and literal word, in lower and in
//! capitalised case, in every position of [`POSITIONS`] and [`STATEMENTS`]:
//! the parser's keyword slots must pass the verifier, and `clickhouse-format`
//! must print the re-cased text as it prints the input. A word re-cased where
//! ClickHouse reads a name (`columns(1)`, `* EXCEPT null`) changes what the
//! oracle prints, since names keep their case.
//!
//! The oracle is `CHAIRLIFT_CLICKHOUSE_FORMAT`, by default `clickhouse-format`
//! on `PATH`; without it the test skips, unless `CHAIRLIFT_REQUIRE_CORPUS` is
//! set. `mise run test:keyword-positions` runs it alone.
//!
//! To cover a new position, add a template. In [`POSITIONS`] it is an
//! expression in which `{w}` stands for the word, parsed as an expression and
//! judged as `SELECT 0, <expression>`, so that a word that begins it is not
//! read as `SELECT`'s `DISTINCT` or `ALL`. In [`STATEMENTS`] it is a whole
//! statement, parsed and judged as one, in which `{w}` stands for the word
//! and `{n}` for the word where only a name can stand, which must then never
//! be a keyword slot, whatever the oracle says.

use std::ops::Range;

use chairlift::keyword::Keyword;
use chairlift::lexer::Mode;
use chairlift::oracle::Judgement;
use chairlift::oracle::Oracle;
use chairlift::oracle::judge;
use chairlift::syntax::Entry;
use chairlift::syntax::SyntaxKind;
use chairlift::syntax::parse_as;
use chairlift::verify::verify;

/// Expressions with `{w}` where the word goes.
#[rustfmt::skip]
const POSITIONS: &[&str] = &[
    // Names and calls.
    "{w}", "t.{w}", "db.t.{w}", "{w}.x", "{w}.{w}", "{w}(x)", "{w}(x, y)", "{w}()",
    "{w}(x)(y)", "{w}(distinct x)", "{w}(*)", "{w}(1)", "{w}(f(x))", "{w}('a', 'b')",
    "{w}((select 1))", "f({w})", "f(x, {w})", "f({w}, {w})", "f({w} as y)", "f(x as {w})",
    "count(distinct {w})", "array({w})", "map({w}, 1)", "tuple({w})", "if({w}, 1, 2)",
    "f(x).{w}", "tuple(1).{w}", "{w}.1", "x.{w}.1", "{w}[1]", "{w}[{w}]", "@@{w}",
    // Aliases.
    "x as {w}", "x {w}", "{w} {w}",
    // Lambdas.
    "arrayMap({w} -> {w}, a)", "arrayMap(({w}, y) -> y, a, b)",
    // Operators.
    "{w} + 1", "1 + {w}", "-{w}", "not {w}", "{w} = 1", "{w} in (1)", "x in {w}",
    "{w} between 1 and 2", "x between {w} and {w}", "{w} is null", "x is {w}", "x is not {w}",
    "{w} like 'a'", "x like {w}", "{w} || 'a'", "{w} * 2", "{w} mod 2", "x {w} 2",
    "x not {w} y", "{w} is not distinct from {w}", "x ? {w} : {w}", "{w} ? 1 : 2",
    "{w} at time zone 'UTC'", "x at {w}",
    // Quantifiers: only one expression or a subquery makes one.
    "x = {w}(1)", "x = {w}(1, 2)", "x = {w}()", "x = {w}(y as z)", "x < {w}(y, z) + 1",
    "x = {w}((select 1))", "x = {w}((select 1), 2)", "x = {w} (select 1)", "x like {w}(y)",
    "x like {w}(y, z)", "x is distinct from {w}(y)", "x ~ {w}(y)",
    // Collections, and brackets that hold a query only where ClickHouse reads
    // one.
    "[{w}]", "({w}, 1)", "({w})", "{{'a': {w}}}", "({w} - 1)", "({w} [1])", "({w} in (1))",
    "({w} and x)", "({w} is null)", "x in ({w} - 1)", "({w} (1))", "({w} (x))", "({w} (x, y))",
    // CASE, INTERVAL, parameters.
    "case {w} when {w} then {w} else {w} end", "case when {w} then 1 end",
    "interval {w} day", "interval 1 {w}", "{{{w}:String}}", "{{x:{w}}}",
    // Column matchers and transformers.
    "{w}.*", "t.{w}.*", "columns({w})", "columns(a, {w})", "columns('a') apply {w}",
    "columns('a') except ({w})", "* except ({w})", "* except {w}", "* except ({w}, a)",
    "* except strict {w}", "* replace (1 as {w})", "* replace ({w} as x)", "* apply {w}",
    "* apply({w})", "* apply({w}) apply {w}", "* apply {w}(0.5)", "* apply(x -> {w})",
    "* replace ({w} + 1 as x)", "columns(a.{w})", "columns({w}) apply {w}",
    // Windows.
    "f(x) over {w}", "f(x) over ({w})",
    // Functions with syntax of their own.
    "cast(x as {w})", "cast(x, '{w}')", "dateAdd({w}, 1, d)", "dateDiff({w}, a, b)",
    "extract({w} from d)", "substring({w} from 1)", "position({w} in s)", "trim({w})",
    "trim(both {w} from s)",
    // Data types.
    "x::{w}", "{w}::String", "x::Nullable({w})", "x::Map(String, {w})", "x::{w}(10)",
    "x::{w}(3, 'UTC')", "x::Enum8('{w}' = 1)", "cast(x as Tuple({w} String))",
    "x::Tuple({w} String, b {w})", "x::AggregateFunction({w}, String)",
    "x::AggregateFunction(1, {w}, String)", "x::SimpleAggregateFunction({w}, String)",
    "x::AggregateFunction({w}(0.5), String)",
    "x::JSON({w} String)",
];

/// Statements with `{w}`, or `{n}` in a name position, where the word goes.
const STATEMENTS: &[&str] = &[
    // SELECT, upper case but for the word.
    "SELECT {w} FROM t",
    "SELECT a, {w} FROM t",
    "SELECT a AS {w} FROM t",
    "SELECT a {w} FROM t",
    "SELECT DISTINCT {w} FROM t",
    "SELECT ({w}) FROM t",
    "SELECT ({w}, 1) FROM t",
    "SELECT {w}(1) FROM t",
    "SELECT * FROM {w}",
    "SELECT * FROM {w}.t",
    "SELECT * FROM db.{w}",
    "SELECT * FROM {w}(1)",
    "SELECT * FROM t AS {w}",
    "SELECT * FROM t {w}",
    "SELECT * FROM t {w} WHERE 1",
    "SELECT * FROM t AS x ({w})",
    "WITH {w} AS (SELECT 1) SELECT * FROM {w}",
    "WITH 1 AS {w} SELECT {w}",
    "SELECT * FROM a JOIN b USING ({w})",
    "SELECT * FROM a JOIN b USING {w}",
    "SELECT * FROM a JOIN b ON {w} = 1",
    "SELECT * FROM a JOIN {w} ON 1",
    "SELECT * FROM t ARRAY JOIN {w}",
    "SELECT * FROM t PREWHERE {w}",
    "SELECT * FROM t WHERE {w}",
    "SELECT * FROM t WHERE {w} = 1",
    "SELECT * FROM t GROUP BY {w}",
    "SELECT * FROM t GROUP BY a, {w}",
    "SELECT * FROM t HAVING {w}",
    "SELECT * FROM t QUALIFY {w}",
    "SELECT * FROM t WINDOW {w} AS ()",
    "SELECT * FROM t ORDER BY {w}",
    "SELECT * FROM t ORDER BY {w} DESC",
    "SELECT * FROM t ORDER BY a, {w}",
    "SELECT x FROM t ORDER BY x WITH FILL INTERPOLATE ({w})",
    "SELECT * FROM t LIMIT {w}",
    "SELECT * FROM t LIMIT 1 BY {w}",
    "SELECT * FROM t LIMIT 1 OFFSET {w}",
    "SELECT * FROM t SETTINGS {w} = 1",
    "SELECT * FROM t FORMAT {w}",
    "SELECT * FROM t LIMIT {w} BY a",
    "SELECT * FROM t OFFSET {w}",
    "SELECT * FROM t AS {w} FINAL",
    "SELECT * FROM t1 {w} JOIN t2 ON 1",
    "SELECT * FROM (SELECT 1) {w}",
    "SELECT * FROM numbers(1) {w}",
    "WITH {w} SELECT 1",
    "FROM {w} SELECT 1",
    "FROM t {w} SELECT 1",
    "SELECT {w} FROM t UNION ALL SELECT 1",
    "SELECT * FROM (SELECT {w} FROM t)",
    "SELECT * FROM t WHERE a IN (SELECT {w} FROM u)",
    "CREATE TABLE t ENGINE = Memory AS SELECT a FROM u {w}",
    "CREATE MATERIALIZED VIEW v TO t AS SELECT a FROM u {w}",
    "CREATE TABLE t (a Int8, PROJECTION p (SELECT {w})) ENGINE = MergeTree ORDER BY a",
    "CREATE TABLE t (a Int8, PROJECTION p (SELECT a GROUP BY {w})) ENGINE = MergeTree ORDER BY a",
    "CREATE TABLE t (a Int8, PROJECTION p (SELECT a ORDER BY {w})) ENGINE = MergeTree ORDER BY a",
    // CREATE, lower case so that its keyword slots change when re-cased;
    // every word stands where a name goes.
    "create table {n}.{n} on cluster {n} (a UInt8) engine = {n} settings {n} = 1 format {n}",
    "create table t (a UInt8, index {n} a type {n} granularity 1, projection {n} (SELECT a), constraint {n} check a > 0) engine = MergeTree order by a",
    "create table t (a UInt8 codec({n}) statistics({n}), b UInt8) engine = MergeTree order by a ttl a group by a set {n} = max(b)",
    "create table t ({n} UInt8) engine = Memory",
    "create table t (a {n}) engine = Memory",
    "create materialized view {n}.{n} refresh every 1 hour depends on {n}.{n} to {n}.{n} as SELECT 1",
];

/// Statements where a word in a name position is rightly a keyword: a
/// column may leave out its type before `AUTO_INCREMENT`, which is then no
/// type's name.
const NAME_EXCEPTIONS: &[&str] = &["create table t (a auto_increment) engine = Memory"];

/// Words that are literals rather than keywords, or both.
const LITERAL_WORDS: [&str; 7] = ["true", "false", "null", "inf", "infinity", "nan", "unknown"];

/// `template` with `{w}` and `{n}` replaced by `word`, and `{{` and `}}` by
/// braces; and where the word went, as byte ranges, all of them and those
/// of `{n}`.
fn fill(template: &str, word: &str) -> (String, Vec<Range<usize>>, Vec<Range<usize>>) {
    let mut out = String::new();
    let mut at = Vec::new();
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(c) = rest.chars().next() {
        let placeholder = ["{w}", "{n}"]
            .into_iter()
            .find_map(|p| rest.strip_prefix(p).map(|tail| (p, tail)));
        if let Some((placeholder, tail)) = placeholder {
            let range = out.len()..out.len() + word.len();
            if placeholder == "{n}" {
                names.push(range.clone());
            }
            at.push(range);
            out.push_str(word);
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("{{").or_else(|| rest.strip_prefix("}}")) {
            out.push(c);
            rest = tail;
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    (out, at, names)
}

/// A position filled with one word.
struct Case {
    text: String,
    /// Where the word is in `text`.
    word: Vec<Range<usize>>,
    /// Where the word stands for a name.
    names: Vec<Range<usize>>,
    /// The index of the template in [`POSITIONS`], then [`STATEMENTS`].
    position: usize,
    /// Whether the template is a whole statement.
    statement: bool,
}

fn cases() -> Vec<Case> {
    let words = Keyword::ALL
        .iter()
        .map(|k| k.as_str().to_ascii_lowercase())
        .chain(LITERAL_WORDS.map(str::to_owned));
    let templates = POSITIONS
        .iter()
        .map(|t| (t, false))
        .chain(STATEMENTS.iter().map(|t| (t, true)));
    let templates: Vec<(&&str, bool)> = templates.collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut cases = Vec::new();
    for word in words {
        let mut capitalised = word.clone();
        capitalised[..1].make_ascii_uppercase();
        for variant in [word, capitalised] {
            for (position, (template, statement)) in templates.iter().enumerate() {
                let (text, word, names) = fill(template, &variant);
                if seen.insert(text.clone()) {
                    cases.push(Case {
                        text,
                        word,
                        names,
                        position,
                        statement: *statement,
                    });
                }
            }
        }
    }
    cases
}

/// The case parsed and re-cased: a failure, or the re-cased text.
fn check(case: &Case) -> (Option<String>, Option<String>) {
    let text = case.text.as_str();
    let entry = if case.statement {
        Entry::File
    } else {
        Entry::Expression
    };
    let parse = parse_as(text, Mode::Sql, entry);
    let recased = parse.recased();
    if let Err(e) = verify(text, &recased, Mode::Sql, parse.facts()) {
        return (
            Some(format!("{text}: re-casing fails verification: {e}")),
            None,
        );
    }
    let name_slot = parse
        .syntax()
        .descendants_with_tokens()
        .filter_map(rowan::NodeOrToken::into_token)
        .any(|t| {
            matches!(t.kind(), SyntaxKind::Keyword | SyntaxKind::LiteralWord)
                && case
                    .names
                    .iter()
                    .any(|r| r.start == usize::from(t.text_range().start()))
        });
    if name_slot && !NAME_EXCEPTIONS.iter().any(|e| e.eq_ignore_ascii_case(text)) {
        return (Some(format!("{text}: a name is a keyword slot")), None);
    }
    (None, Some(recased))
}

#[test]
fn keyword_slots_keep_what_clickhouse_reads() {
    let program = std::env::var_os("CHAIRLIFT_CLICKHOUSE_FORMAT")
        .unwrap_or_else(|| "clickhouse-format".into());
    let oracle = Oracle::new(program);
    if !oracle.is_available() {
        assert!(
            std::env::var_os("CHAIRLIFT_REQUIRE_CORPUS").is_none_or(|v| v.is_empty()),
            "clickhouse-format is unavailable, and CHAIRLIFT_REQUIRE_CORPUS is set"
        );
        eprintln!("skipping: clickhouse-format is unavailable");
        return;
    }
    let cases = cases();
    // Parsing and verifying are independent per case; spread them over the
    // CPUs, since there are many.
    let jobs = std::thread::available_parallelism().map_or(1, usize::from);
    let chunk = cases.len().div_ceil(jobs);
    let checked: Vec<(Option<String>, Option<String>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = cases
            .chunks(chunk)
            .map(|chunk| scope.spawn(move || chunk.iter().map(check).collect::<Vec<_>>()))
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("no panic"))
            .collect()
    });
    let mut failures = Vec::new();
    let mut judged = Vec::new();
    let mut template_judged = vec![false; POSITIONS.len() + STATEMENTS.len()];
    for (case, (failure, recased)) in cases.iter().zip(checked) {
        failures.extend(failure);
        let Some(recased) = recased else {
            continue;
        };
        let text = case.text.as_str();
        // Only a re-cased word is news in an expression; its template's own
        // keywords are judged once. A statement is judged whenever it
        // changes: what the word is can change how the rest parses.
        let word_recased = case
            .word
            .iter()
            .any(|r| recased[r.clone()] != text[r.clone()]);
        let judge_it = if case.statement {
            recased != text
        } else {
            word_recased || (recased != text && !template_judged[case.position])
        };
        if judge_it {
            template_judged[case.position] = true;
            judged.push((case, recased));
        }
    }
    let texts: Vec<String> = judged
        .iter()
        .flat_map(|(case, recased)| {
            if case.statement {
                [case.text.clone(), recased.clone()]
            } else {
                [
                    format!("SELECT 0, {}", case.text),
                    format!("SELECT 0, {recased}"),
                ]
            }
        })
        .collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let verdicts = oracle.canonical(&refs);
    let mut name_cases = 0;
    let mut name_cases_same = 0;
    for ((case, recased), pair) in judged.iter().zip(verdicts.chunks(2)) {
        let judgement = judge(&pair[0], &pair[1]);
        if !case.names.is_empty() {
            name_cases += 1;
            name_cases_same += usize::from(judgement == Judgement::Same);
        }
        let case = case.text.as_str();
        match judgement {
            Judgement::Same | Judgement::Unjudgeable(_) => {}
            Judgement::Differs { input, output } => failures.push(format!(
                "{case} re-cased to {recased}: the oracle prints {:?} and {:?}",
                input.trim(),
                output.trim()
            )),
            Judgement::OutputRejected(why) => failures.push(format!(
                "{case} re-cased to {recased}: the oracle rejects it: {}",
                why.message
            )),
        }
    }
    eprintln!(
        "{} cases, {} re-cased and judged, {} failures",
        cases.len(),
        judged.len(),
        failures.len()
    );
    // The statements with names in them are lower case, so all are judged;
    // most must be ones ClickHouse accepts, or the names are not tested.
    assert!(
        name_cases_same * 10 > name_cases * 9,
        "{name_cases_same} of {name_cases} statements with names judged the same"
    );
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures
            .iter()
            .take(200)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
