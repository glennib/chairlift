//! ClickHouse's own lexer as a test oracle.
//!
//! `clickhouse_lexer_driver.cpp` is compiled together with the fetched
//! `src/Parsers/Lexer.cpp` in its standalone configuration, which needs clang
//! (GCC rejects the standalone header). `CHAIRLIFT_CLICKHOUSE_CXX` names the
//! compiler; the default is `clang++`.

use std::fs;
use std::io::Write as _;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use chairlift::lexer::Token;
use chairlift::lexer::TokenKind;

use crate::corpus;

/// A token as ClickHouse names it: its `TokenType` name and byte range.
pub type ChToken = (&'static str, Range<usize>);

pub struct Oracle {
    binary: PathBuf,
    /// `TokenType` names, indexed by their value.
    names: Vec<&'static str>,
}

impl Oracle {
    /// Builds the oracle, or reports a skip and returns `None` when the
    /// ClickHouse source or the compiler is unavailable.
    pub fn build() -> Option<Oracle> {
        let Some(clickhouse) = corpus::clickhouse() else {
            corpus::skip("the ClickHouse checkout");
            return None;
        };
        let src = clickhouse.join("src");
        let lexer_cpp = src.join("Parsers/Lexer.cpp");
        let driver =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/clickhouse_lexer_driver.cpp");
        let names = token_names(&fs::read_to_string(src.join("Parsers/Lexer.h")).ok()?);

        let out_dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
        let binary = out_dir.join("clickhouse-lexer");
        if is_fresh(&binary, &[&lexer_cpp, &driver]) {
            return Some(Oracle { binary, names });
        }

        let cxx = std::env::var("CHAIRLIFT_CLICKHOUSE_CXX").unwrap_or_else(|_| "clang++".into());
        // Unique per process: nextest runs tests in parallel processes.
        let scratch = out_dir.join(format!("clickhouse-lexer.{}", std::process::id()));
        let status = Command::new(&cxx)
            .args([
                "-O2",
                "-std=c++20",
                "-DLEXER_STANDALONE_BUILD",
                "-fno-exceptions",
            ])
            .arg("-I")
            .arg(&src)
            .arg(&driver)
            .arg(&lexer_cpp)
            .arg("-o")
            .arg(&scratch)
            .status();
        match status {
            Ok(status) if status.success() => {}
            Ok(status) => panic!("{cxx} failed to build ClickHouse's lexer: {status}"),
            Err(_) => {
                corpus::skip(&format!("the C++ compiler `{cxx}`"));
                return None;
            }
        }
        fs::rename(&scratch, &binary).expect("the built oracle can be moved into place");
        Some(Oracle { binary, names })
    }

    /// ClickHouse's tokens for each input, with adjacent whitespace merged into
    /// one token as chairlift does.
    ///
    /// # Panics
    ///
    /// If an input contains a NUL byte, which separates inputs for the driver.
    pub fn lex(&self, inputs: &[&str]) -> Vec<Vec<ChToken>> {
        assert!(inputs.iter().all(|i| !i.contains('\0')));
        let mut child = Command::new(&self.binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("the oracle runs");
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let joined = inputs.join("\0");
        let writer = std::thread::spawn(move || stdin.write_all(joined.as_bytes()));
        let output = child.wait_with_output().expect("the oracle finishes");
        writer.join().unwrap().expect("the oracle reads its input");
        assert!(
            output.status.success(),
            "the oracle failed: {}",
            output.status
        );

        let stdout = String::from_utf8(output.stdout).expect("the oracle prints ASCII");
        let mut results = Vec::with_capacity(inputs.len());
        let mut current = Vec::new();
        for line in stdout.lines() {
            if line == "-" {
                results.push(merge_whitespace(std::mem::take(&mut current)));
                continue;
            }
            let mut fields = line.split(' ').map(|f| f.parse::<usize>().unwrap());
            let (kind, begin, end) = (
                fields.next().unwrap(),
                fields.next().unwrap(),
                fields.next().unwrap(),
            );
            current.push((self.names[kind], begin..end));
        }
        assert_eq!(results.len(), inputs.len());
        results
    }
}

/// chairlift's tokens in ClickHouse's terms.
///
/// # Panics
///
/// If a token has a kind ClickHouse does not have.
pub fn as_clickhouse(tokens: &[Token]) -> Vec<ChToken> {
    tokens
        .iter()
        .map(|t| (clickhouse_name(t.kind()), t.range()))
        .collect()
}

fn clickhouse_name(kind: TokenKind) -> &'static str {
    kind.clickhouse_name()
        .unwrap_or_else(|| panic!("{kind:?} has no ClickHouse counterpart"))
}

fn merge_whitespace(tokens: Vec<ChToken>) -> Vec<ChToken> {
    let mut merged: Vec<ChToken> = Vec::with_capacity(tokens.len());
    for (name, range) in tokens {
        if let Some((last_name, last_range)) = merged.last_mut()
            && name == "Whitespace"
            && *last_name == "Whitespace"
            && last_range.end == range.start
        {
            last_range.end = range.end;
            continue;
        }
        merged.push((name, range));
    }
    merged
}

/// The names in `APPLY_FOR_TOKENS`, in order. They are leaked so that tokens
/// can hold them without an allocation each; there are about sixty.
fn token_names(header: &str) -> Vec<&'static str> {
    let mut names = Vec::new();
    let mut inside = false;
    for line in header.lines() {
        if line.starts_with("#define APPLY_FOR_TOKENS") {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(rest) = line.trim_start().strip_prefix("M(") {
            let name = rest.split(')').next().unwrap().trim();
            names.push(&*Box::leak(name.into()));
        }
        if !line.trim_end().ends_with('\\') {
            break;
        }
    }
    assert!(names.contains(&"EndOfStream"), "parsed Lexer.h");
    names
}

fn is_fresh(binary: &Path, sources: &[&Path]) -> bool {
    let modified = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok();
    let Some(built) = modified(binary) else {
        return false;
    };
    sources
        .iter()
        .all(|s| modified(s).is_some_and(|m| m <= built))
}
