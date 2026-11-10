//! Locating the fetched corpora (`scripts/fetch-corpus.sh`).
//!
//! Tests that need a corpus skip when it is absent, unless
//! `CHAIRLIFT_REQUIRE_CORPUS` is set: then a missing corpus fails the test, so
//! a CI job that fetched it cannot pass by skipping.

use std::path::Path;
use std::path::PathBuf;

/// `$CHAIRLIFT_CORPUS_DIR`, by default `corpus/` in the repository.
pub fn corpus_dir() -> PathBuf {
    std::env::var_os("CHAIRLIFT_CORPUS_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus"),
        PathBuf::from,
    )
}

/// The ClickHouse checkout, if it has been fetched.
pub fn clickhouse() -> Option<PathBuf> {
    let dir = corpus_dir().join("ClickHouse");
    dir.join("src/Parsers").is_dir().then_some(dir)
}

/// Reports that `what` is unavailable: a skip, or a failure when the corpus is
/// required.
pub fn skip(what: &str) {
    assert!(
        std::env::var_os("CHAIRLIFT_REQUIRE_CORPUS").is_none_or(|v| v.is_empty()),
        "{what} is unavailable, and CHAIRLIFT_REQUIRE_CORPUS is set"
    );
    eprintln!("skipping: {what} is unavailable");
}
