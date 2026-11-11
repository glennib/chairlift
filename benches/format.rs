//! `chairlift::format` end to end, on inputs in the repository: about 1 MB
//! of generated SQL with realistic nesting (see `tests/support/doc_sql.rs`),
//! and the vendored fixtures, real migrations and templates, each formatted
//! on its own as the CLI does.
//!
//! CI compares this benchmark with the base commit's in the same job
//! (`scripts/bench-compare.sh`), so it reads nothing that only a local
//! checkout or the corpus job has.

use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use chairlift::Options;
use chairlift::lexer::Mode;
use criterion::Criterion;
use criterion::Throughput;
use criterion::criterion_group;
use criterion::criterion_main;

mod api {
    pub use chairlift::doc;
    pub use chairlift::lexer;
}

#[path = "../tests/support/doc_sql.rs"]
mod doc_sql;

/// The fixtures' SQL files and templates, in path order, with their options.
fn fixtures() -> Vec<(String, Options)> {
    fn has_extension(path: &Path, extension: &str) -> bool {
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(extension))
    }
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("the fixtures are readable") {
            let path = entry.expect("a fixture entry is readable").path();
            if path.is_dir() {
                walk(&path, out);
            } else if has_extension(&path, "sql")
                || (has_extension(&path, "j2")
                    && path
                        .file_stem()
                        .is_some_and(|stem| has_extension(Path::new(stem), "sql")))
            {
                out.push(path);
            }
        }
    }
    let mut paths = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut paths,
    );
    paths.sort();
    paths
        .iter()
        .map(|p| {
            let text = fs::read_to_string(p).expect("a fixture is UTF-8");
            let options = Options::default().with_jinja(Mode::for_path(p) == Mode::Jinja);
            (text, options)
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let (_, sample) = doc_sql::script(&mut doc_sql::Rng::new(1), 200);
    let statements = 200 * (1 << 20) / sample.len() + 1;
    let (_, generated) = doc_sql::script(&mut doc_sql::Rng::new(2026), statements);
    let options = Options::default();
    chairlift::format(&generated, &options).expect("the generated SQL formats");

    let mut group = c.benchmark_group("format");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));
    group.throughput(Throughput::Bytes(generated.len() as u64));
    group.bench_function("generated_1mb", |b| {
        b.iter(|| chairlift::format(black_box(&generated), &options));
    });

    let files = fixtures();
    for (text, options) in &files {
        chairlift::format(text, options).expect("every fixture formats");
    }
    let bytes: usize = files.iter().map(|(t, _)| t.len()).sum();
    group.throughput(Throughput::Bytes(bytes as u64));
    group.bench_function("fixtures", |b| {
        b.iter(|| {
            for (text, options) in &files {
                black_box(chairlift::format(black_box(text), options).ok());
            }
        });
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
