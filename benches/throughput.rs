//! Throughput benchmarks over the vendored fixtures.
//!
//! `scan_lines` is a floor: no stage of the formatter can process the input
//! faster than counting its newlines.

use std::fs;
use std::hint::black_box;
use std::path::Path;

use criterion::Criterion;
use criterion::Throughput;
use criterion::criterion_group;
use criterion::criterion_main;

/// Every fixture file, concatenated in path order.
fn fixtures() -> String {
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(dir).expect("fixture directory is readable") {
            let path = entry.expect("fixture entry is readable").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "sql" || e == "j2") {
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
        .map(|p| fs::read_to_string(p).expect("fixture is UTF-8"))
        .collect()
}

fn scan_lines(c: &mut Criterion) {
    let input = fixtures();
    let mut group = c.benchmark_group("fixtures");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_function("scan_lines", |b| {
        b.iter(|| black_box(&input).bytes().filter(|&b| b == b'\n').count());
    });
    group.finish();
}

criterion_group!(benches, scan_lines);
criterion_main!(benches);
