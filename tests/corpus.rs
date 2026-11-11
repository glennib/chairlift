//! The lexer, the splitter, the formatter and the parser over every file of
//! every corpus that is present: the vendored fixtures, the fetched
//! ClickHouse stateless tests and dbt projects, and the local-only
//! directories in `CHAIRLIFT_LOCAL_CORPORA`.

#[path = "support/clickhouse_lexer.rs"]
mod clickhouse_lexer;
#[path = "support/corpus.rs"]
mod corpus;

use std::fs;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;

use chairlift::Error;
use chairlift::Options;
use chairlift::lexer::Mode;
use chairlift::lexer::Token;
use chairlift::lexer::jinja;
use chairlift::lexer::jinja::TagKind;
use chairlift::lexer::tokenize;
use chairlift::split::DataFormat;
use chairlift::split::split;
use chairlift::syntax::Entry;
use chairlift::syntax::SyntaxKind;
use chairlift::syntax::SyntaxNode;
use chairlift::syntax::parse_as;
use chairlift::verify::Facts;
use chairlift::verify::verify;
use rowan::NodeOrToken;
use rowan::WalkEvent;

struct Corpus {
    name: String,
    files: Vec<PathBuf>,
    /// Every file is a template, whatever its name (dbt models are `.sql`).
    all_templates: bool,
}

impl Corpus {
    fn mode(&self, path: &Path) -> Mode {
        if self.all_templates {
            Mode::Jinja
        } else {
            Mode::for_path(path)
        }
    }
}

/// Every corpus that is present. A missing fetched corpus is a skip (or a
/// failure under `CHAIRLIFT_REQUIRE_CORPUS`); a missing local one is ignored.
fn corpora() -> Vec<Corpus> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut corpora = vec![Corpus {
        name: "fixtures".into(),
        files: sql_files(&manifest.join("tests/fixtures")),
        all_templates: false,
    }];
    match corpus::clickhouse() {
        Some(clickhouse) => corpora.push(Corpus {
            name: "stateless".into(),
            files: sql_files(&clickhouse.join("tests/queries/0_stateless")),
            all_templates: false,
        }),
        None => corpus::skip("the ClickHouse stateless tests"),
    }
    let dbt = corpus::corpus_dir().join("dbt");
    if dbt.is_dir() {
        corpora.push(Corpus {
            name: "dbt".into(),
            files: sql_files(&dbt),
            all_templates: true,
        });
    } else {
        corpus::skip("the dbt projects");
    }
    if let Some(local) = std::env::var_os("CHAIRLIFT_LOCAL_CORPORA") {
        for (i, dir) in std::env::split_paths(&local)
            .filter(|d| d.is_dir())
            .enumerate()
        {
            corpora.push(Corpus {
                name: format!("local corpus {}", i + 1),
                files: sql_files(&dir),
                all_templates: false,
            });
        }
    }
    corpora
}

/// The `*.sql` and `*.sql.j2` files under `dir`, sorted, skipping `.git` and
/// `target` directories. (`*.j2.sql` is covered by `*.sql`.)
fn sql_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() {
                if name != ".git" && name != "target" {
                    walk(&path, out);
                }
            } else if path.extension().is_some_and(|e| e == "sql")
                || name.to_str().is_some_and(|n| n.ends_with(".sql.j2"))
            {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    files
}

/// The file's text, or `None` if it is not UTF-8.
fn read(path: &Path) -> Option<String> {
    String::from_utf8(fs::read(path).expect("corpus file is readable")).ok()
}

/// The lexer's guarantees: tokens are non-empty and contiguous and cover the
/// source.
fn check_lossless(source: &str, tokens: &[Token]) -> Result<(), String> {
    let mut pos = 0;
    for token in tokens {
        if token.start() != pos {
            return Err(format!("{token:?} does not start at {pos}"));
        }
        if token.end() <= token.start() {
            return Err(format!("{token:?} is empty"));
        }
        pos = token.end();
    }
    if pos != source.len() {
        return Err(format!("tokens end at {pos} of {}", source.len()));
    }
    let joined: String = tokens.iter().map(|t| t.text(source)).collect();
    if joined != source {
        return Err("tokens do not concatenate to the source".into());
    }
    Ok(())
}

/// Every file round-trips in both modes, whatever its own mode is.
#[test]
fn every_corpus_file_round_trips() {
    let mut failures = Vec::new();
    for corpus in corpora() {
        let (mut files, mut templates, mut bytes, mut not_utf8, mut with_errors) = (0, 0, 0, 0, 0);
        for path in &corpus.files {
            let Some(source) = read(path) else {
                not_utf8 += 1;
                continue;
            };
            for mode in [Mode::Sql, Mode::Jinja] {
                let tokens = tokenize(&source, mode);
                if let Err(e) = check_lossless(&source, &tokens) {
                    failures.push(format!("{} ({mode:?}): {e}", path.display()));
                }
                if mode == corpus.mode(path) && tokens.iter().any(|t| t.kind().is_error()) {
                    with_errors += 1;
                }
            }
            files += 1;
            templates += usize::from(corpus.mode(path) == Mode::Jinja);
            bytes += source.len();
        }
        eprintln!(
            "{}: {files} files ({templates} templates, {bytes} bytes) round-trip in both modes; \
             {not_utf8} not UTF-8; {with_errors} with error tokens in their own mode",
            corpus.name
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every file formats without a verification failure, or is left alone for a
/// lexer error or a line comment the rendering can make run on, and
/// formatting is idempotent.
#[test]
fn every_corpus_file_formats_idempotently() {
    let mut failures = Vec::new();
    for corpus in corpora() {
        let (mut formatted, mut changed, mut lex_errors, mut refused) = (0, 0, 0, 0);
        for path in &corpus.files {
            let Some(source) = read(path) else {
                continue;
            };
            let options = Options::default().with_jinja(corpus.mode(path) == Mode::Jinja);
            match chairlift::format(&source, &options) {
                Ok(output) => {
                    formatted += 1;
                    changed += usize::from(output != source);
                    match chairlift::format(&output, &options) {
                        Ok(again) if again == output => {}
                        Ok(_) => failures.push(format!("{}: not idempotent", path.display())),
                        Err(e) => failures.push(format!("{}: second pass: {e}", path.display())),
                    }
                }
                Err(Error::Lex(_)) => lex_errors += 1,
                Err(Error::Template(_)) => refused += 1,
                Err(e) => failures.push(format!("{}: {e}", path.display())),
            }
        }
        eprintln!(
            "{}: {formatted} files verify and are idempotent, {changed} of them changed; \
             {lex_errors} left alone for a lexer error, {refused} for a line comment the \
             rendering can make run on",
            corpus.name
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The text after each `FORMAT name` of an `INSERT` (no `;` between the two
/// words), past an optional line break, up to the first blank line: text
/// that may be inline data. Found from the lexer's words, not by the
/// splitter, so a head the splitter misreads still counts.
fn maybe_data(source: &str, mode: Mode) -> Vec<&str> {
    let mut found = Vec::new();
    let mut insert = false;
    for token in tokenize(source, mode) {
        let word = |w: &str| {
            token.kind() == chairlift::lexer::TokenKind::BareWord
                && token.text(source).eq_ignore_ascii_case(w)
        };
        if token.kind() == chairlift::lexer::TokenKind::Semicolon {
            insert = false;
        } else if word("INSERT") {
            insert = true;
        } else if insert && word("FORMAT") {
            let rest = &source[token.end()..];
            let name = rest.trim_start_matches([' ', '\t']);
            let name_len = name
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(name.len());
            let after = name[name_len..].trim_start_matches([' ', '\t', '\x0c', '\r']);
            if name_len == 0 || name.starts_with("Values") || after.starts_with([';', ')']) {
                continue;
            }
            let data = after.strip_prefix('\n').unwrap_or(after);
            found.push(&data[..data.find("\n\n").unwrap_or(data.len())]);
        }
    }
    found
}

/// The inline data of every `INSERT` of `text`, as the splitter finds it, in
/// order.
fn split_data(text: &str, mode: Mode) -> Vec<&str> {
    let split = split(text, mode);
    split
        .statements()
        .iter()
        .filter_map(|s| s.data)
        .map(|d| split.tokens()[d.token].text(text))
        .collect()
}

/// Formatting keeps every byte of text that may be an `INSERT`'s inline
/// data in a format other than `Values`, however the head before it reads,
/// and the output holds the same data as the input, of every format, where
/// the splitter finds it.
#[test]
fn every_corpus_file_keeps_its_data() {
    let mut failures = Vec::new();
    for corpus in corpora() {
        let mut regions = 0;
        for path in &corpus.files {
            let Some(source) = read(path) else {
                continue;
            };
            let options = Options::default().with_jinja(corpus.mode(path) == Mode::Jinja);
            let Ok(output) = chairlift::format(&source, &options) else {
                continue;
            };
            let mode = corpus.mode(path);
            if split_data(&source, mode) != split_data(&output, mode) {
                failures.push(format!(
                    "{}: the data the splitter finds changed",
                    path.display()
                ));
            }
            for data in maybe_data(&source, corpus.mode(path)) {
                regions += 1;
                if !output.contains(data) {
                    failures.push(format!("{}: {:?} changed", path.display(), excerpt(data)));
                }
                // The verifier sees a trailing space added to the data.
                let Some(line_end) = data.find('\n') else {
                    continue;
                };
                let at = data.as_ptr() as usize - source.as_ptr() as usize + line_end;
                let changed = format!("{} {}", &source[..at], &source[at..]);
                if verify(&source, &changed, corpus.mode(path), &Facts::new()).is_ok() {
                    failures.push(format!(
                        "{}: a space added to {:?} verifies",
                        path.display(),
                        excerpt(data)
                    ));
                }
            }
        }
        eprintln!("{}: {regions} possible data regions kept", corpus.name);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn excerpt(text: &str) -> &str {
    &text[..text.char_indices().nth(60).map_or(text.len(), |(i, _)| i)]
}

/// Every template that formats renders the same SQL before and after, in
/// every combination of its branches and loop count (see
/// `chairlift::template`).
#[cfg(feature = "template")]
#[test]
fn every_corpus_template_renders_the_same_sql() {
    use chairlift::template::compare_sql;
    use chairlift::template::render;

    let mut failures = Vec::new();
    for corpus in corpora() {
        let (mut templates, mut variants, mut rendered) = (0, 0, 0);
        for path in &corpus.files {
            if corpus.mode(path) != Mode::Jinja {
                continue;
            }
            let Some(source) = read(path) else {
                continue;
            };
            let Ok(output) = chairlift::format(&source, &Options::default().with_jinja(true))
            else {
                continue;
            };
            templates += 1;
            let (before, after) = (render(&source), render(&output));
            if before.branches != after.branches {
                failures.push(format!("{}: the branches differ", path.display()));
                continue;
            }
            for (a, b) in before.variants.iter().zip(&after.variants) {
                variants += 1;
                match (&a.text, &b.text) {
                    (Ok(x), Ok(y)) => {
                        rendered += 1;
                        if let Err(e) = compare_sql(x, y) {
                            failures.push(format!("{} {}: {e}", path.display(), a.label));
                        }
                    }
                    (Err(_), Err(_)) => {}
                    _ => failures.push(format!(
                        "{} {}: only one side renders",
                        path.display(),
                        a.label
                    )),
                }
            }
        }
        eprintln!(
            "{}: {templates} templates, {variants} variants, {rendered} rendered on both sides",
            corpus.name
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The splitter's tokens cover every file, and the counts of statements and
/// of inline data are reported per corpus.
#[test]
fn every_corpus_file_splits() {
    let mut failures = Vec::new();
    for corpus in corpora() {
        let (mut files, mut statements, mut values, mut other) = (0, 0, 0, 0);
        for path in &corpus.files {
            let Some(source) = read(path) else {
                continue;
            };
            let split = split(&source, corpus.mode(path));
            if let Err(e) = check_lossless(&source, split.tokens()) {
                failures.push(format!("{}: {e}", path.display()));
            }
            files += 1;
            statements += split.statements().len();
            for data in split.statements().iter().filter_map(|s| s.data) {
                match data.format {
                    DataFormat::Values => values += 1,
                    DataFormat::Other => other += 1,
                }
            }
        }
        eprintln!(
            "{}: {files} files, {statements} statements, {values} with VALUES data, \
             {other} with data in another format",
            corpus.name
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The kinds of a tree's nodes and tokens in order, and the first node that
/// starts or ends with trivia, or has no tokens, if there is one: one walk,
/// so that it stays cheap on large trees.
fn shape(root: &SyntaxNode) -> (Vec<SyntaxKind>, Option<String>) {
    let mut kinds = Vec::new();
    let mut bad = None;
    // Open nodes, and whether each has seen a token.
    let mut open: Vec<(SyntaxKind, bool)> = Vec::new();
    let mut last_is_trivia = false;
    for event in root.preorder_with_tokens() {
        match event {
            WalkEvent::Enter(NodeOrToken::Node(node)) => {
                kinds.push(node.kind());
                open.push((node.kind(), false));
            }
            WalkEvent::Enter(NodeOrToken::Token(token)) => {
                kinds.push(token.kind());
                last_is_trivia = token.kind().is_trivia();
                for (kind, seen) in open.iter_mut().rev() {
                    if *seen {
                        break;
                    }
                    *seen = true;
                    if last_is_trivia && *kind != SyntaxKind::Root {
                        bad.get_or_insert_with(|| format!("{kind:?} starts with trivia"));
                    }
                }
            }
            WalkEvent::Leave(NodeOrToken::Node(node)) => {
                let seen = open.pop().is_some_and(|(_, seen)| seen);
                if node.kind() != SyntaxKind::Root && (last_is_trivia || !seen) {
                    bad.get_or_insert_with(|| {
                        format!(
                            "{:?} at {:?} ends with trivia",
                            node.kind(),
                            node.text_range()
                        )
                    });
                }
            }
            WalkEvent::Leave(NodeOrToken::Token(_)) => {}
        }
    }
    (kinds, bad)
}

/// Every file parses, as a file and with the expression probe, into a tree
/// whose text is the file, with no node starting or ending with trivia; the
/// keyword slots re-cased pass the verifier, and the re-cased text parses
/// to the same tree.
#[test]
fn every_corpus_file_parses_losslessly() {
    let mut failures = Vec::new();
    for corpus in corpora() {
        let (mut files, mut statements, mut probed, mut slots) = (0, 0, 0, 0);
        for path in &corpus.files {
            let Some(source) = read(path) else {
                continue;
            };
            let mode = corpus.mode(path);
            for entry in [Entry::File, Entry::SelectListProbe] {
                let parse = parse_as(&source, mode, entry);
                let root = parse.syntax();
                if root.text() != source.as_str() {
                    failures.push(format!("{} ({entry:?}): not lossless", path.display()));
                    continue;
                }
                let (kinds, bad) = shape(&root);
                if let Some(bad) = bad {
                    failures.push(format!("{} ({entry:?}): {bad}", path.display()));
                }
                let recased = parse.recased();
                if let Err(e) = verify(&source, &recased, mode, parse.facts()) {
                    failures.push(format!("{} ({entry:?}): re-cased: {e}", path.display()));
                }
                if recased != source && shape(&parse_as(&recased, mode, entry).syntax()).0 != kinds
                {
                    failures.push(format!(
                        "{} ({entry:?}): the re-cased text parses differently",
                        path.display()
                    ));
                }
                if entry == Entry::SelectListProbe {
                    probed += kinds
                        .iter()
                        .filter(|k| **k == SyntaxKind::ExpressionProbe)
                        .count();
                    slots += kinds.iter().filter(|k| **k == SyntaxKind::Keyword).count();
                } else {
                    statements += kinds
                        .iter()
                        .filter(|k| **k == SyntaxKind::Statement)
                        .count();
                }
            }
            files += 1;
        }
        eprintln!(
            "{}: {files} files, {statements} statements, {probed} probed as expression lists \
             with {slots} keyword slots",
            corpus.name
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_corpus_file_lexes_as_clickhouse_does() {
    let Some(oracle) = clickhouse_lexer::Oracle::build() else {
        return;
    };
    let mut sources = Vec::new();
    for corpus in corpora() {
        for path in corpus.files {
            if let Some(source) = read(&path).filter(|s| !s.contains('\0')) {
                sources.push((path, source));
            }
        }
    }

    let mut failures = Vec::new();
    let mut compared = 0;
    // Batches keep the oracle's output, about 15 bytes per token, in check.
    let mut rest = &sources[..];
    while !rest.is_empty() {
        let mut size = 0;
        let count = rest
            .iter()
            .take_while(|(_, s)| {
                size += s.len();
                size < 4 << 20
            })
            .count()
            .max(1);
        let (batch, tail) = rest.split_at(count);
        rest = tail;
        let inputs: Vec<&str> = batch.iter().map(|(_, s)| s.as_str()).collect();
        for ((path, source), clickhouse) in batch.iter().zip(oracle.lex(&inputs)) {
            let chairlift = clickhouse_lexer::as_clickhouse(&tokenize(source, Mode::Sql));
            if let Some(i) = (0..chairlift.len().max(clickhouse.len()))
                .find(|&i| chairlift.get(i) != clickhouse.get(i))
            {
                failures.push(format!(
                    "{}: token {i}: ClickHouse {:?}, chairlift {:?}",
                    path.display(),
                    clickhouse.get(i),
                    chairlift.get(i)
                ));
            }
            compared += 1;
        }
    }
    eprintln!("{compared} files lex in SQL mode as ClickHouse lexes them");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// What minijinja's tokenizer found in a template.
struct MinijinjaTags {
    /// `{{ … }}` and `{% … %}` tags, other than the delimiters of raw blocks.
    tags: Vec<(TagKind, Range<usize>)>,
    /// The bytes minijinja accounted for: template data and tags. Jinja
    /// comments, raw delimiters and whitespace removed by `-` markers are not
    /// covered.
    covered: Vec<Range<usize>>,
    /// minijinja stopped at an error; everything before this offset was
    /// tokenized.
    error_after: Option<usize>,
}

fn minijinja_tags(source: &str) -> MinijinjaTags {
    use minijinja::machinery::Token as Mj;

    let mut found = MinijinjaTags {
        tags: Vec::new(),
        covered: Vec::new(),
        error_after: None,
    };
    let mut open = None;
    let mut boundary = 0;
    let config = minijinja::syntax::SyntaxConfig::default();
    for item in minijinja::machinery::tokenize(source, false, config) {
        let Ok((token, span)) = item else {
            found.error_after = Some(boundary);
            break;
        };
        let range = span.start_offset as usize..span.end_offset as usize;
        match token {
            Mj::VariableStart => open = Some((TagKind::Expr, range.start)),
            Mj::BlockStart => open = Some((TagKind::Block, range.start)),
            Mj::VariableEnd | Mj::BlockEnd => {
                let (kind, start) = open.take().expect("a tag end follows its start");
                found.tags.push((kind, start..range.end));
                found.covered.push(start..range.end);
                boundary = range.end;
            }
            Mj::TemplateData(_) => {
                boundary = range.end;
                found.covered.push(range);
            }
            _ => {}
        }
    }
    found
}

/// chairlift's tags, anywhere in the token stream, in source order.
fn chairlift_tags(source: &str) -> Vec<jinja::Tag> {
    tokenize(source, Mode::Jinja)
        .iter()
        .flat_map(|token| {
            jinja::tags(token.text(source)).map(move |tag| jinja::Tag {
                range: tag.range.start + token.start()..tag.range.end + token.start(),
                ..tag
            })
        })
        .collect()
}

/// Compares the tags chairlift finds with what minijinja finds, up to the
/// first minijinja error: the same `{{ }}` and `{% %}` spans, and every byte
/// minijinja skips is in a Jinja comment, a raw block, or whitespace removed
/// by a `-` marker.
fn compare_with_minijinja(source: &str) -> Result<(usize, bool), String> {
    let theirs = minijinja_tags(source);
    let cutoff = theirs.error_after.unwrap_or(source.len());
    let ours = chairlift_tags(source);

    let our_tags: Vec<(TagKind, Range<usize>)> = ours
        .iter()
        .filter(|t| matches!(t.kind, TagKind::Expr | TagKind::Block) && t.range.end <= cutoff)
        .map(|t| (t.kind, t.range.clone()))
        .collect();
    let their_tags: Vec<_> = theirs
        .tags
        .iter()
        .filter(|(_, r)| r.end <= cutoff)
        .cloned()
        .collect();
    if our_tags != their_tags {
        let i = (0..our_tags.len().max(their_tags.len()))
            .find(|&i| our_tags.get(i) != their_tags.get(i))
            .unwrap();
        return Err(format!(
            "tag {i}: minijinja {:?}, chairlift {:?}",
            their_tags.get(i),
            our_tags.get(i)
        ));
    }

    let mut accounted = vec![false; cutoff];
    for range in &theirs.covered {
        accounted[range.start.min(cutoff)..range.end.min(cutoff)].fill(true);
    }
    for tag in ours.iter().filter(|t| t.range.end <= cutoff) {
        match tag.kind {
            TagKind::Comment => {
                if accounted[tag.range.clone()].iter().any(|&a| a) {
                    return Err(format!(
                        "comment {:?} overlaps minijinja's tokens",
                        tag.range
                    ));
                }
                accounted[tag.range.clone()].fill(true);
            }
            TagKind::Raw => accounted[tag.range.clone()].fill(true),
            TagKind::Expr | TagKind::Block => {}
        }
    }
    let mut pos = 0;
    for c in source[..cutoff].chars() {
        if !accounted[pos] && !c.is_whitespace() {
            return Err(format!(
                "minijinja skipped {c:?} at {pos}, outside any comment or raw block"
            ));
        }
        pos += c.len_utf8();
    }
    Ok((our_tags.len(), theirs.error_after.is_some()))
}

/// chairlift finds the same Jinja tags as minijinja, the renderer chum uses.
/// Plain SQL files are included: they are text too, and the renderer would
/// find tags in them.
#[test]
fn tags_agree_with_minijinja() {
    let mut failures = Vec::new();
    for corpus in corpora() {
        let (mut files, mut templates, mut tags, mut stopped) = (0, 0, 0, 0);
        for path in &corpus.files {
            let Some(source) = read(path) else {
                continue;
            };
            match compare_with_minijinja(&source) {
                Ok((n, error)) => {
                    tags += n;
                    stopped += usize::from(error);
                    if error && std::env::var_os("CHAIRLIFT_LIST_MINIJINJA_ERRORS").is_some() {
                        eprintln!("minijinja stopped in {}", path.display());
                    }
                }
                Err(e) => failures.push(format!("{}: {e}", path.display())),
            }
            files += 1;
            templates += usize::from(corpus.mode(path) == Mode::Jinja);
        }
        eprintln!(
            "{}: {files} files ({templates} templates), {tags} tags agree with minijinja; \
             minijinja stopped at an error in {stopped}",
            corpus.name
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn minijinja_comparison_covers_every_tag_kind() {
    let source = "a {{- x -}} b {# c #} {% raw %}{{ y }}{% endraw %} '{%+ if z +%}' x_{{ w }}";
    assert_eq!(compare_with_minijinja(source), Ok((3, false)));
    assert_eq!(
        compare_with_minijinja("{{ x }} {{ ? }} {{ y }}"),
        Ok((1, true))
    );
    for tricky in [
        "{{ \"}}\" }}",
        "{% set s = '%}' %}",
        "{{ {'a': {'b': 1}} }}",
        "{{ f(')') }}",
        "{{ 'a\\'}}' }}",
        "{{-}} {{--}} {%-%} {#-#} {##} {#}#}",
        "{%- raw -%} ' {%- endraw %}",
        "{%raw%}{%endraw%}",
        "{% raw %}{% raw %}{% endraw %}",
        "{% rawx %}",
        "'\\{{ \"'\" }}'",
        "-- {{ x\n }}\n/* {{ \"*/\" }} */ $$ {{ \"$$\" }} $$",
        "events{% if v2 %}_v2{% endif %}",
    ] {
        assert!(
            matches!(compare_with_minijinja(tricky), Ok((_, false))),
            "{tricky:?}: {:?}",
            compare_with_minijinja(tricky)
        );
    }
}
