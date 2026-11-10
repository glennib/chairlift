//! Throughput benchmarks over the vendored fixtures and, when it has been
//! fetched, the ClickHouse stateless corpus.
//!
//! Each file is lexed on its own, as the formatter will: concatenated, an
//! unclosed string or tag in one file would swallow every file after it.
//!
//! `scan_lines` is a floor: no stage of the formatter can process the input
//! faster than counting its newlines. `lex` counts the tokens of a
//! [`Lexer`](chairlift::lexer::Lexer); `tokenize` also collects them, as
//! later stages do.

use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use chairlift::lexer::Lexer;
use chairlift::lexer::Mode;
use chairlift::lexer::tokenize;
use criterion::Criterion;
use criterion::Throughput;
use criterion::criterion_group;
use criterion::criterion_main;

/// The UTF-8 files under `dir` whose names end in one of `suffixes`, in path
/// order.
fn read_files(dir: &Path, suffixes: &[&str]) -> Vec<String> {
    fn walk(dir: &Path, suffixes: &[&str], out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("corpus directory is readable") {
            let path = entry.expect("corpus entry is readable").path();
            if path.is_dir() {
                walk(&path, suffixes, out);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| suffixes.iter().any(|s| n.ends_with(s)))
            {
                out.push(path);
            }
        }
    }
    let mut paths = Vec::new();
    walk(dir, suffixes, &mut paths);
    paths.sort();
    paths
        .iter()
        .filter_map(|p| String::from_utf8(fs::read(p).expect("corpus file is readable")).ok())
        .collect()
}

fn bench_files(c: &mut Criterion, name: &str, files: &[String], mode: Mode) {
    let bytes: usize = files.iter().map(String::len).sum();
    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Bytes(bytes as u64));
    if bytes > 1 << 20 {
        group.sample_size(20);
        group.measurement_time(Duration::from_secs(10));
    }
    group.bench_function("scan_lines", |b| {
        b.iter(|| {
            files
                .iter()
                .map(|f| black_box(f).bytes().filter(|&b| b == b'\n').count())
                .sum::<usize>()
        });
    });
    group.bench_function("lex", |b| {
        b.iter(|| {
            files
                .iter()
                .map(|f| Lexer::new(black_box(f), mode).count())
                .sum::<usize>()
        });
    });
    group.bench_function("tokenize", |b| {
        b.iter(|| {
            for f in files {
                black_box(tokenize(black_box(f), mode));
            }
        });
    });
    group.finish();
}

fn fixtures(c: &mut Criterion) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let files = read_files(&dir, &[".sql", ".sql.j2"]);
    bench_files(c, "fixtures", &files, Mode::Jinja);
}

/// The stateless `.sql` files in SQL mode, and the same files in Jinja mode
/// for the cost of looking for tags.
fn stateless(c: &mut Criterion) {
    // A test run (`cargo nextest run --all-targets`) runs every benchmark once,
    // in a debug build; 35 MB is too slow for that, so only `cargo bench`
    // (which passes `--bench`) reads the corpus.
    if !std::env::args().any(|a| a == "--bench") {
        return;
    }
    let corpus = std::env::var_os("CHAIRLIFT_CORPUS_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus"),
        PathBuf::from,
    );
    let dir = corpus.join("ClickHouse/tests/queries/0_stateless");
    if !dir.is_dir() {
        eprintln!(
            "skipping the stateless corpus: {} is missing",
            dir.display()
        );
        return;
    }
    let files = read_files(&dir, &[".sql"]);
    bench_files(c, "stateless", &files, Mode::Sql);
    bench_files(c, "stateless_jinja", &files, Mode::Jinja);
}

criterion_group!(benches, fixtures, stateless);
criterion_main!(benches);
