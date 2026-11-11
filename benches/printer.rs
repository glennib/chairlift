//! Building, checking and printing a document for about 1 MB of SQL with
//! realistic nesting (see `tests/support/doc_sql.rs`): queries with
//! subqueries, calls, `CASE`, lambdas, conditions and comments.
//!
//! Lexing is left out; `throughput` measures it. Before the measurements,
//! the benchmark prints how many allocations one build and one print make.

use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::hint::black_box;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use chairlift::doc::PrintOptions;
use chairlift::lexer::Mode;
use chairlift::split::split;
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

/// Counts allocations, to report them.
struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

// SAFETY: forwards every call to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: the caller upholds `alloc`'s contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller upholds `dealloc`'s contract.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED.fetch_add(new_size, Ordering::Relaxed);
        // SAFETY: the caller upholds `realloc`'s contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Allocations and bytes allocated while running `f`.
fn allocations<T>(f: impl FnOnce() -> T) -> (T, usize, usize) {
    let (count, bytes) = (
        ALLOCATIONS.load(Ordering::Relaxed),
        ALLOCATED.load(Ordering::Relaxed),
    );
    let out = f();
    (
        out,
        ALLOCATIONS.load(Ordering::Relaxed) - count,
        ALLOCATED.load(Ordering::Relaxed) - bytes,
    )
}

fn bench(c: &mut Criterion) {
    let mut rng = doc_sql::Rng::new(2026);
    let (_, sample) = doc_sql::script(&mut doc_sql::Rng::new(1), 200);
    let statements = 200 * (1 << 20) / sample.len() + 1;
    let (queries, source) = doc_sql::script(&mut rng, statements);
    let split = split(&source, Mode::Sql);
    let tokens = split.tokens();
    let options = PrintOptions::default();

    let (doc, build_count, build_bytes) =
        allocations(|| doc_sql::document(&queries, &source, tokens, Mode::Sql));
    let (out, print_count, print_bytes) = allocations(|| doc.print(&options));
    doc.check_tokens()
        .expect("the layout prints every token once");
    eprintln!(
        "printer: {} bytes of source, {} tokens, {} bytes printed; \
         build: {build_count} allocations ({build_bytes} bytes), \
         print: {print_count} allocations ({print_bytes} bytes)",
        source.len(),
        tokens.len(),
        out.len()
    );

    let mut group = c.benchmark_group("printer");
    group.throughput(Throughput::Bytes(source.len() as u64));
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function("build", |b| {
        b.iter(|| doc_sql::document(&queries, black_box(&source), tokens, Mode::Sql));
    });
    group.bench_function("check_tokens", |b| {
        b.iter(|| black_box(&doc).check_tokens());
    });
    group.bench_function("print", |b| b.iter(|| black_box(&doc).print(&options)));
    group.bench_function("build_check_print", |b| {
        b.iter(|| {
            let doc = doc_sql::document(&queries, black_box(&source), tokens, Mode::Sql);
            doc.check_tokens().expect("every token once");
            doc.print(&options)
        });
    });
    // References on the same text, for timings taken on a busy machine:
    // lexing and splitting, and the formatter as it stands (splitting,
    // parsing, the safe cleanup and verifying).
    group.bench_function("reference_split", |b| {
        b.iter(|| chairlift::split::split(black_box(&source), Mode::Sql));
    });
    group.bench_function("reference_format", |b| {
        b.iter(|| chairlift::format(black_box(&source), &chairlift::Options::default()));
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
