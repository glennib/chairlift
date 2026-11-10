//! The lexer over every file of every corpus that is present: the vendored
//! fixtures, the fetched ClickHouse stateless tests and dbt projects, and the
//! local-only directories in `CHAIRLIFT_LOCAL_CORPORA`.

#[path = "support/clickhouse_lexer.rs"]
mod clickhouse_lexer;
#[path = "support/corpus.rs"]
mod corpus;

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use chairlift::lexer::Token;
use chairlift::lexer::tokenize;

struct Corpus {
    name: String,
    files: Vec<PathBuf>,
}

/// Every corpus that is present. A missing fetched corpus is a skip (or a
/// failure under `CHAIRLIFT_REQUIRE_CORPUS`); a missing local one is ignored.
fn corpora() -> Vec<Corpus> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut corpora = vec![Corpus {
        name: "fixtures".into(),
        files: sql_files(&manifest.join("tests/fixtures")),
    }];
    match corpus::clickhouse() {
        Some(clickhouse) => corpora.push(Corpus {
            name: "stateless".into(),
            files: sql_files(&clickhouse.join("tests/queries/0_stateless")),
        }),
        None => corpus::skip("the ClickHouse stateless tests"),
    }
    let dbt = corpus::corpus_dir().join("dbt");
    if dbt.is_dir() {
        corpora.push(Corpus {
            name: "dbt".into(),
            files: sql_files(&dbt),
        });
    } else {
        corpus::skip("the dbt projects");
    }
    if let Some(local) = std::env::var_os("CHAIRLIFT_LOCAL_CORPORA") {
        for dir in std::env::split_paths(&local).filter(|d| d.is_dir()) {
            corpora.push(Corpus {
                name: format!("local {}", dir.display()),
                files: sql_files(&dir),
            });
        }
    }
    corpora
}

/// The `.sql` and `.j2` files under `dir`, sorted, skipping `.git` and
/// `target` directories.
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
            } else if path.extension().is_some_and(|e| e == "sql" || e == "j2") {
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

#[test]
fn every_corpus_file_round_trips() {
    let mut failures = Vec::new();
    for corpus in corpora() {
        let (mut files, mut bytes, mut not_utf8, mut with_errors) = (0, 0, 0, 0);
        for path in &corpus.files {
            let Some(source) = read(path) else {
                not_utf8 += 1;
                continue;
            };
            let tokens = tokenize(&source);
            if let Err(e) = check_lossless(&source, &tokens) {
                failures.push(format!("{}: {e}", path.display()));
            }
            if tokens.iter().any(|t| t.kind().is_error()) {
                with_errors += 1;
            }
            files += 1;
            bytes += source.len();
        }
        eprintln!(
            "{}: {files} files ({bytes} bytes) round-trip; {not_utf8} not UTF-8; \
             {with_errors} with error tokens",
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
            let chairlift = clickhouse_lexer::as_clickhouse(&tokenize(source));
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
    eprintln!("{compared} files lex as ClickHouse lexes them");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
