//! The docs-examples corpus: the SQL code blocks of ClickHouse's reference
//! documentation, kept where `clickhouse-format` accepts them.
//!
//! The documentation is Mintlify `.mdx` under `docs/reference` of the
//! ClickHouse checkout that `scripts/fetch-corpus.sh` fetches. Its SQL blocks
//! hold statements, but also function signatures (`f(x[, y])`), placeholders,
//! client prompts and result tables, so each block is judged by the oracle
//! first, as a whole and then statement by statement, and only what it
//! accepts is written out: one `.sql` file per block that keeps a statement,
//! at `<page>.mdx/<line>.sql`, where `<line>` is the line of the block's
//! opening fence. The corpus is then an ordinary directory of SQL files.
//!
//! The extraction is pinned by the checkout's revision and the oracle's
//! version, recorded in [`STAMP`] with a fingerprint of this file,
//! [`EXTRACTOR`]; it is redone only when one of them changes.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use chairlift::lexer::Mode;
use chairlift::lexer::TokenKind;
use chairlift::oracle::Judgement;
use chairlift::oracle::Oracle;
use chairlift::oracle::Pair;
use chairlift::oracle::RejectionClass;
use chairlift::split::split;

/// The file in the extracted corpus that records what it was extracted
/// from. Not a `.sql` file, so the corpus walkers skip it.
pub const STAMP: &str = "stamp.txt";

/// A fingerprint of this file's source (64-bit FNV-1a), so that any change
/// to the extraction redoes an extracted corpus. Changes in the library
/// that alter what is extracted, such as in the splitter, do not.
pub const EXTRACTOR: u64 = fnv1a(include_str!("docs.rs").as_bytes());

const fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    hash
}

/// A fenced code block of SQL.
#[derive(Debug, PartialEq, Eq)]
pub struct Block {
    /// The 1-based line of its opening fence.
    pub line: usize,
    /// Its content, without the fences and without the opening fence's
    /// indent.
    pub text: String,
}

/// The fenced code blocks of `markdown` whose info string names SQL
/// (```` ```sql ````, ```` ``` SQL title="Query" ````), in order.
///
/// Fences follow CommonMark: three or more backticks or tildes, closed by a
/// run of the same character at least as long with nothing after it, or by
/// the end of the text. A fence may be indented, as inside a list or a
/// component; that indent is removed from the block's lines.
#[must_use]
pub fn sql_blocks(markdown: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut lines = markdown.lines().enumerate();
    while let Some((n, line)) = lines.next() {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let rest = &line[indent..];
        let Some(fence) = rest.chars().next().filter(|c| *c == '`' || *c == '~') else {
            continue;
        };
        let length = rest.len() - rest.trim_start_matches(fence).len();
        if length < 3 {
            continue;
        }
        let info = rest[length..].trim();
        // A backtick fence's info string cannot hold a backtick.
        if fence == '`' && info.contains('`') {
            continue;
        }
        let language = info
            .split(|c: char| c.is_whitespace() || c == '{')
            .next()
            .unwrap_or("");
        let mut text = String::new();
        for (_, line) in lines.by_ref() {
            let trimmed = line.trim();
            if trimmed.len() >= length && trimmed.chars().all(|c| c == fence) {
                break;
            }
            let strip = line.len() - line.trim_start_matches(' ').len();
            text.push_str(&line[strip.min(indent)..]);
            text.push('\n');
        }
        if language.eq_ignore_ascii_case("sql") {
            blocks.push(Block { line: n + 1, text });
        }
    }
    blocks
}

/// What one extraction found.
#[derive(Debug, Default)]
pub struct Stats {
    /// `.mdx` files read.
    pub pages: usize,
    /// Those with at least one SQL block.
    pub pages_with_sql: usize,
    /// SQL blocks.
    pub blocks: usize,
    /// Blocks the oracle accepts as a whole.
    pub blocks_accepted: usize,
    /// Blocks with at least one statement it accepts: the files written.
    pub blocks_kept: usize,
    /// Statements of all blocks, as chairlift's splitter finds them.
    pub statements: usize,
    /// Statements it accepts.
    pub statements_accepted: usize,
    /// Statements it rejects, per [`RejectionClass`].
    pub rejected: [usize; RejectionClass::ALL.len()],
}

impl Stats {
    /// The counts as a short report, one per line.
    #[must_use]
    pub fn report(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "pages: {} ({} with SQL blocks)",
            self.pages, self.pages_with_sql
        );
        let _ = writeln!(
            out,
            "SQL blocks: {} ({} accepted whole, {} kept)",
            self.blocks, self.blocks_accepted, self.blocks_kept
        );
        let _ = writeln!(
            out,
            "statements: {} ({} accepted)",
            self.statements, self.statements_accepted
        );
        let rejected: Vec<String> = RejectionClass::ALL
            .iter()
            .zip(self.rejected)
            .filter(|(_, n)| *n > 0)
            .map(|(class, n)| format!("{} {n}", class.name()))
            .collect();
        let _ = writeln!(out, "statements rejected: {}", rejected.join(", "));
        out
    }
}

/// The part of `block` the oracle accepts, given whether it accepts each of
/// the statements the splitter finds in it: the block as it is if it accepts
/// them all, else each accepted statement with the comments and blank lines
/// before it and the comments after it on its last line, and the trivia
/// after the last statement. `None` if it accepts none.
#[must_use]
pub fn kept(block: &str, accepted: &[bool]) -> Option<String> {
    let split = split(block, Mode::Sql);
    assert_eq!(split.statements().len(), accepted.len());
    if !accepted.contains(&true) {
        return None;
    }
    if !accepted.contains(&false) {
        return Some(block.to_owned());
    }
    let tokens = split.tokens();
    let mut out = String::new();
    let mut previous = 0;
    for (statement, &ok) in split.statements().iter().zip(accepted) {
        // A comment after the `;` on its line comments on this statement,
        // though it lies before the next one.
        let mut last = statement.tokens.end;
        while let Some(token) = tokens.get(last) {
            match token.kind() {
                TokenKind::Whitespace if !token.text(block).contains('\n') => {}
                TokenKind::LineComment | TokenKind::BlockComment => {}
                _ => break,
            }
            last += 1;
        }
        let end = tokens[last - 1].end();
        if ok {
            out.push_str(&block[previous..end]);
        }
        previous = end;
    }
    out.push_str(&block[previous..]);
    Some(out.trim_start_matches(['\n', '\r']).to_owned())
}

/// The stamp an extraction of `pages` (the `docs/reference` directory of a
/// ClickHouse checkout) with `clickhouse_format` writes.
///
/// # Errors
///
/// If the checkout's revision or the oracle's version cannot be read.
pub fn stamp(pages: &Path, clickhouse_format: &Path) -> Result<String, String> {
    let git = Command::new("git")
        .arg("-C")
        .arg(pages)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !git.status.success() {
        return Err(format!(
            "cannot find the revision of {}: {}",
            pages.display(),
            String::from_utf8_lossy(&git.stderr).trim()
        ));
    }
    let revision = String::from_utf8_lossy(&git.stdout).trim().to_owned();
    // `clickhouse-format` has no `--version`, but names its version in the
    // error about the option; `clickhouse --version` prints it.
    let run = Command::new(clickhouse_format)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run {}: {e}", clickhouse_format.display()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let version = release(&text)
        .ok_or_else(|| format!("{} --version names no version", clickhouse_format.display()))?;
    Ok(format!(
        "# Written by `corpus --extract-docs`, which redoes the extraction when\n\
         # this file would change.\n\
         docs: ClickHouse {revision}, docs/reference\n\
         oracle: clickhouse-format {version}\n\
         extractor: {EXTRACTOR:016x}\n"
    ))
}

/// The last `version 1.2.3` that `text` names, as `1.2.3`.
fn release(text: &str) -> Option<&str> {
    text.rmatch_indices("version ").find_map(|(at, word)| {
        let rest = &text[at + word.len()..];
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let number = rest[..end].trim_end_matches('.');
        number.contains('.').then_some(number)
    })
}

/// Extracts the SQL blocks of the `.mdx` files under `pages` into `target`,
/// keeping what `oracle` accepts, unless `target` already holds an
/// extraction with the same `stamp`. Returns the counts, or `None` when the
/// extraction was up to date.
///
/// `target` is replaced as a whole: the blocks are written to a sibling
/// directory first, which is then moved into place.
///
/// # Errors
///
/// If a file cannot be read or written.
pub fn extract(
    pages: &Path,
    target: &Path,
    oracle: &Oracle,
    stamp: &str,
) -> Result<Option<Stats>, String> {
    if fs::read_to_string(target.join(STAMP)).is_ok_and(|s| s == stamp) {
        return Ok(None);
    }
    let mut stats = Stats::default();
    let mut blocks = Vec::new();
    for page in mdx_files(pages) {
        let text = fs::read_to_string(&page)
            .map_err(|e| format!("cannot read {}: {e}", page.display()))?;
        stats.pages += 1;
        let found = sql_blocks(&text);
        if !found.is_empty() {
            stats.pages_with_sql += 1;
        }
        let relative = page.strip_prefix(pages).unwrap_or(&page).to_path_buf();
        blocks.extend(found.into_iter().map(|b| (relative.clone(), b)));
    }
    stats.blocks = blocks.len();

    let pairs: Vec<Pair<'_>> = blocks
        .iter()
        .map(|(_, b)| Pair {
            input: &b.text,
            output: &b.text,
            mode: Mode::Sql,
        })
        .collect();
    let reports = oracle.judge_pairs(&pairs);

    let staging = sibling(target, "tmp");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)
        .map_err(|e| format!("cannot create {}: {e}", staging.display()))?;
    for ((page, block), report) in blocks.iter().zip(&reports) {
        if report.whole == Judgement::Same {
            stats.blocks_accepted += 1;
        }
        let accepted: Vec<bool> = report
            .statements
            .iter()
            .map(|s| s.judgement == Judgement::Same)
            .collect();
        stats.statements += accepted.len();
        for statement in &report.statements {
            match &statement.judgement {
                Judgement::Same => stats.statements_accepted += 1,
                Judgement::Unjudgeable(why) => stats.rejected[why.class as usize] += 1,
                // The same text on both sides cannot differ, but count it.
                Judgement::Differs { .. } | Judgement::OutputRejected(_) => {
                    stats.rejected[RejectionClass::Other as usize] += 1;
                }
            }
        }
        let Some(mut text) = kept(&block.text, &accepted) else {
            continue;
        };
        if !text.ends_with('\n') {
            text.push('\n');
        }
        let dir = staging.join(page);
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let file = dir.join(format!("{:05}.sql", block.line));
        fs::write(&file, text).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
        stats.blocks_kept += 1;
    }
    let path = staging.join(STAMP);
    fs::write(&path, stamp).map_err(|e| format!("cannot write {}: {e}", path.display()))?;

    let old = sibling(target, "old");
    let _ = fs::remove_dir_all(&old);
    if target.exists() {
        fs::rename(target, &old).map_err(|e| format!("cannot move {}: {e}", target.display()))?;
    }
    fs::rename(&staging, target).map_err(|e| format!("cannot move {}: {e}", staging.display()))?;
    let _ = fs::remove_dir_all(&old);
    Ok(Some(stats))
}

/// `dir` with `.suffix` added to its name.
fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_owned();
    name.push(".");
    name.push(suffix);
    dir.with_file_name(name)
}

/// The `.mdx` files under `dir`, sorted.
fn mdx_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "mdx") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_blocks_are_found_by_their_info_string() {
        let page = "\
# Title

```sql title=\"Query\"
SELECT 1;
```

```response
┌─1─┐
```

``` SQL
SELECT 2
```

- item

  ```sql
  SELECT 3
    FROM t;
  ```

````sql
```
````

~~~sql
SELECT 4
";
        let blocks = sql_blocks(page);
        let found: Vec<(usize, &str)> = blocks.iter().map(|b| (b.line, b.text.as_str())).collect();
        assert_eq!(
            found,
            [
                (3, "SELECT 1;\n"),
                (11, "SELECT 2\n"),
                (17, "SELECT 3\n  FROM t;\n"),
                (22, "```\n"),
                (26, "SELECT 4\n"),
            ]
        );
    }

    #[test]
    fn other_languages_and_inline_backticks_are_not_blocks() {
        assert_eq!(
            sql_blocks("```sqlite\nx\n```\n```text\nSELECT 1\n```\n"),
            []
        );
        assert_eq!(sql_blocks("``sql``\n`` `sql` ``\n"), []);
        // A block of another language hides a fence inside it.
        assert_eq!(sql_blocks("````md\n```sql\nSELECT 1\n```\n````\n"), []);
    }

    #[test]
    fn the_oracle_release_is_found_in_what_it_prints() {
        assert_eq!(
            release("unrecognised option '--version' … (version 26.10.2.103 (official build))"),
            Some("26.10.2.103")
        );
        assert_eq!(
            release("ClickHouse local version 26.10.2.103 (official build).\n"),
            Some("26.10.2.103")
        );
        assert_eq!(release("no version here"), None);
    }

    #[test]
    fn kept_drops_the_rejected_statements_with_their_comments() {
        let block = "-- one\nSELECT 1;\n\n-- signature\nf(x[, y]);\n-- three\nSELECT 3; -- end\n";
        assert_eq!(kept(block, &[true, true, true]).as_deref(), Some(block));
        assert_eq!(
            kept(block, &[true, false, true]).as_deref(),
            Some("-- one\nSELECT 1;\n-- three\nSELECT 3; -- end\n")
        );
        assert_eq!(
            kept(block, &[false, false, true]).as_deref(),
            Some("-- three\nSELECT 3; -- end\n")
        );
        assert_eq!(kept(block, &[false, false, false]), None);
    }

    #[test]
    fn kept_keeps_a_trailing_comment_with_its_statement() {
        let block = "SELECT 1; -- first\nf(x[, y]); /* signature */ -- too\nSELECT 3;\n";
        assert_eq!(
            kept(block, &[true, false, true]).as_deref(),
            Some("SELECT 1; -- first\nSELECT 3;\n")
        );
        assert_eq!(
            kept(block, &[false, true, false]).as_deref(),
            Some("f(x[, y]); /* signature */ -- too\n")
        );
    }
}
