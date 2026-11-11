# chairlift: implementation progress

Tracks the implementation of milestones M0–M8 from `MILESTONES.md`.
M9 (chum integration) is not part of this run.

## Status

| Milestone | State | Notes |
| --- | --- | --- |
| M0. Repository scaffolding | done | CI green |
| M1. Lossless lexer | done | reviewed; CI green |
| M2. Verifier, splitter, safe-cleanup CLI | done | review findings fixed |
| M3. Parser core | implemented | all three parts and the part 1 review fixes are in; review of the `SELECT` and `CREATE` grammars running |
| M4. Formatting rules and printer | in progress | part 1: document IR and printer |
| M5. Jinja templates | not started | |
| M6. CLI polish and performance | not started | |
| M7. Remaining statement kinds | not started | |
| M8. Hardening and first release | not started | |

## Sequencing

M0 to M4 are implemented in order on one stack of commits.
M6 touches mostly the CLI, so it runs beside M5 in a separate workspace and is rebased onto it.
M7 follows M5, and M8 comes last.

Each milestone is reviewed by a second agent that re-runs the acceptance criteria itself
before the milestone is marked done here.
The next milestone starts while that review runs; findings are fixed on top of the stack.

Implementers run each fuzz target for a few minutes only.
Longer fuzz runs happen beside the next milestone, and a crash becomes a fix on top of the stack.

## Open questions: defaults taken

The open questions in `MILESTONES.md` each state what the plan assumes.
Those assumptions are used unless the project owner decides otherwise:

| Milestone | Question | Default taken |
| --- | --- | --- |
| M2 | CRLF files written back as CRLF or LF | LF |
| M2 | Exit code when a lexer error leaves a file unchanged | 2 |
| M4 | `WITH TOTALS` placement | on the `GROUP BY` line when it fits, own line otherwise |
| M5 | Cap on rendered branch combinations in the test harness | 16 per template |
| M6 | `--verify` and Docker | `clickhouse-format` on `PATH` or `--clickhouse-binary` only |

## Environment constraints

- Work is pushed to `main` on `origin` after each milestone's review, without pull requests.
  CI results are recorded per milestone below.
- No release automation runs yet: only CI triggers on a push.
  Release configuration is kept in the repository,
  but publishing to crates.io and tagging `0.1.0` in M8 are left to the project owner.
- The ClickHouse corpus is fetched at `v26.10.2.103-stable`.
  The fetch script takes `CHAIRLIFT_CORPUS_DIR` so that several checkouts can share one copy;
  it defaults to `corpus/` in the repository.
- The private migrations corpus is read from its local path when present and is never vendored.

## Waiting for the project owner

- Snapshot review: `insta` snapshots are committed so that every commit passes its tests.
  They are listed here per milestone until reviewed.
  - M0: the inline snapshot of `--help` in `tests/cli.rs`.
  - M3 part 2: 11 tree-dump snapshots under `src/syntax/grammar/select/snapshots/`.
  - M3 part 3: 10 tree-dump snapshots under `src/syntax/grammar/create/snapshots/`.
  - M3 part 1: 24 tree-dump snapshots under `src/syntax/snapshots/`
    and `src/syntax/grammar/{exprs,types,common,fallback}/snapshots/`.

## Deviations from the plan

- **Lexer rules (M1)**: where the plan's summary of ClickHouse's lexer differs from ClickHouse's source,
  the source is followed.
  The lexer's token stream is compared with ClickHouse's own lexer, compiled from the fetched source, in the tests.
  - A `.` not followed by a digit is a `Dot`; at offset 0 of the input a `.` always starts a number.
  - `1e` at the end of input is a bare word; `1e+` is the number `1e` and a `Plus`.
  - `$` is a bare word only when a word character follows, otherwise a `DollarSign`.
  - A hex or binary string with a wrong character is an error token that runs to the end of input.
  - ASCII and Unicode whitespace are one `Whitespace` token; ClickHouse emits them separately.
- **Expression grammar (M3)**: ClickHouse's parser is followed where the plan's summary differs.
  - `LIKE` and `IN` share the comparison level; `BETWEEN`, then `IS NULL`, bind more loosely;
    `||` sits between the comparisons and `+`.
  - There are 45 restricted words for implicit aliases, not 44,
    and `COMMENT 'x'` at the end of a statement is an exception.
  - Only a literal left operand of `::`
    (and the literal first argument of `CAST`)
    is read as source text, and only whether whitespace follows each comma matters there.
  - `inf`, `infinity` and `nan` are always literals.
    `TRUE` and `FALSE` after `IS` are literals too, so they print lower case.
  - `NOT` in operand position is always the operator, so `not(x)` is re-cased to `NOT(x)`.
  - chairlift parses `a ? b : c ? d : e` and `x -> y -> z`, which ClickHouse rejects.
- **Jinja mode (M2)**: dbt models are templates in files named `.sql`, so the extension alone cannot decide.
  The CLI takes `--jinja auto|on|off`, with `auto` deciding by extension.
- **Lexer throughput (M1)** is measured per file, not over one concatenated input: concatenated,
  an unclosed string in one file swallows every file after it.

## Milestone notes

Filled in as milestones complete: what was built, measured acceptance numbers, known gaps.

### M0

- Every commit passes `mise run ci` (build, clippy, tests, doctests, formatting, `actionlint`, `shellcheck`).
- The corpus fetch gives 12,619 `.sql` and 241 `.sql.j2` stateless files and 38 dbt sample files, matching the plan;
  a repeated fetch does no network calls.
- Benchmarks use `criterion`, for its saved baselines (M6 needs a regression threshold).
- `cargo-fuzz` is declared in `mise.toml`; `fuzz/` is its own workspace with no targets yet.
- The cargo-dist workflow is not checked in, because `dist generate` always adds tag and pull-request triggers.
  `dist generate --mode=ci` creates it when releases are wanted.
  `release-plz.yml` exists but runs only on manual dispatch.
- `CHAIRLIFT_LOCAL_CORPORA` (`:`-separated directories) is documented for the local-only corpora;
  the corpus tool that reads it comes in M2.
- CI: the first run on `main` passed, including the corpus job that fetches and caches the stateless corpus.

### M1

Acceptance, as measured by the implementer and re-measured by the independent review:

| Criterion | Result |
| --- | --- |
| Round trip on every corpus file | holds in both modes: 14 fixtures, 12,866 stateless (9 non-UTF-8 skipped), 38 dbt, 9 local |
| Edge-case table of at least 60 | 142 cases (96 SQL, 46 Jinja) |
| Token kinds agree with ClickHouse | identical streams to ClickHouse's compiled lexer on the edge cases and on 12,926 corpus files |
| Throughput of at least 200 MB/s | about 255 MB/s for `tokenize`, 274 MB/s streaming |
| Fuzzing for 10 minutes | 601 s, 1.94 million runs, no crash |
| minijinja differential | about 3,930 tags, no disagreement, nothing allowlisted |

- The keyword list has 514 words from 666 phrases, as the plan expects.
  `mise run keywords:check` verifies the checked-in list against the fetched source.
- `mise run test:corpus` runs the corpus tests;
  the CI corpus job runs it with `CHAIRLIFT_REQUIRE_CORPUS` so that a missing corpus fails.
- Compiling ClickHouse's standalone lexer for the comparison needs clang.
- The private migrations corpus has 4 files and no template, not the 6 the plan lists.
- Review: no blockers.
  The reviewer's own random comparisons found no difference from ClickHouse's lexer
  (300,000 fragments)
  or from minijinja's tag spans (3 million strings), and no pair that `can_touch` wrongly lets touch
  (7.5 million token triples).
  Its findings were fixed in M2's stack: an unclosed Jinja tag inside a line comment is now an error token,
  the heredoc closer search is linear, and the contents of a `LineComment` token are documented
  (never its `\n`, a CRLF's `\r` stays inside it, and a tag can make it span lines).

### M2

Acceptance, as measured by the implementer (the independent review re-runs these):

| Criterion | Result |
| --- | --- |
| Injected printer bug (`-` and `-1` adjacent) | caught by the verifier, in `tests/verify.rs` |
| Verify and idempotence over every corpus | hold on every file that lexes |
| Oracle canonical forms, allowlist empty | 0 differing, 0 rejected outputs |
| `--check` on the stateless corpus under 5 s | 0.8–0.9 s, single-threaded |
| DESIGN.md example unchanged | unchanged |

Corpus numbers:

| Corpus | Files | Formatted | Lexer errors | Changed | Oracle accepted, all unchanged | Oracle cannot judge |
| --- | --- | --- | --- | --- | --- | --- |
| stateless | 12,875 | 12,851 | 15 (and 9 not UTF-8) | 1,034 | 12,303 | 548 |
| dbt | 38 | 38 | 0 | 25 | 3 | 35 (unrendered Jinja) |
| fixtures | 14 | 14 | 0 | 0 | 8 | 6 (unrendered Jinja) |
| local | 9 | 9 | 0 | 0 | 9 | 0 |

- The fuzz target for `format` found 7 verification failures in Jinja edge cases; each is fixed with a regression test.
- Bytes inside a token are never changed: block comments, strings, heredocs,
  tags and `INSERT` data keep CRLF and trailing spaces,
  so a CRLF file containing one of these comes out with mixed line endings.
  A line comment's trailing `\r` and spaces are trimmed.
- The start and end of the input are left alone where ClickHouse's lexer treats the first or last byte specially
  (`1e`, `1e+`, a leading `.`),
  and `INSERT` data in a non-`VALUES` format that runs to the end of the file gets no final newline.
- Known gaps: forcing blank lines around multi-line statements is left to M4;
  hard links to a formatted file are broken by the rename-based write;
  whether cargo-dist would ship the `corpus` binary is unchecked.
- `mise run corpus` and `mise run corpus:oracle` run the corpus tool; `--csv FILE` writes one row per file.
- Review: acceptance numbers re-measured correctly.
  It found the following, all since fixed:
  - **Blocker**: `INSERT` data is not recognised when the statement starts with a `{{ }}` tag,
    or when a bare word `format` appears earlier in it.
    Cleanup then strips trailing spaces inside the data and the verifier accepts it.
  - `{% raw %}` blocks are not modelled by the verifier's rendered-text check.
  - Whitespace next to a `{{ }}` tag can be removed unchecked, although the tag can render to anything.
  - The keyword re-case exemption accepts any bare word, not only keywords in their canonical case.
  - A read-only file is rewritten.
  - The oracle judges whole files, so one rejected statement hides the file:
    about 11,800 statements in the stateless corpus are not judged.
- A 30-minute fuzz run after the milestone found two more bugs, both fixed:
  `can_touch` let a token ending in `$` touch a word character, which can form a heredoc closer,
  and a `#` comment in a template lost the space that makes it a comment.
- After the fixes each fuzz target ran 15 minutes without a crash
  (about 857,000 and 970,000 runs),
  the corpus and oracle numbers are unchanged,
  and the reviewer's whitespace-mutation harness finds no unsound acceptance on the real templates.
- Rules that came out of the fixes, which the printer in M4 must follow:
  - Whitespace next to a `{{ }}` tag or a raw block stays present or absent as in the input;
    `can_touch` answers no for any pair that includes one.
  - In a template, cleanup changes a run of whitespace only when the run is outside every rendered token
    and its rendered left neighbour is the one it has in the template.
  - In a template with block tags, everything from an `INSERT … FORMAT` to the end of the file is left as it is.
  - A recorded keyword slot may only change into its canonical case,
    and the word must be a ClickHouse keyword or a literal word
    (`true`, `false`, `inf`, `infinity`, `nan` lower case; everything else, `NULL` included, upper).
  - A file without write permission is an error (exit 2) and is not rewritten.
- Known limitation, owned by M5: the verifier models only the render in which every `{% if %}` branch is taken.
- The oracle judges statement by statement where it rejects a whole file.
  On the stateless corpus 270,259 of 273,812 statements are judged (98.7%, up from 93.2%) and none differs.
  The rest are rejected by the oracle itself: 2,115 syntax errors
  (mostly tests of other query dialects), 913 with unrendered Jinja, 257 `INSERT` data, 208 empty queries, 60 other.
  Statements without a tag in a template are judged too.
- The full oracle sweep is slower for it: 4.8 minutes on a loaded machine, against 2.3 before.
  A rejected statement costs a `clickhouse-format` process of its own.
  `clickhouse local` with a formatting function is an untried faster route.
- Oracle blind spot: `clickhouse-format` prints the same `GRANULARITY` for a text index whatever the input says.

### M3

Part 1: parser infrastructure, syntax tree, expression grammar, keyword-slot recording.

- Every corpus file's tree reproduces its input; verify and idempotence hold;
  the oracle judges 270,259 stateless statements the same and none differing.
- `format` parses every statement but still only applies M2's cleanup.
  `--check` over the stateless corpus takes 1.1–1.2 s, against 0.8 s before parsing.
- Expressions, measured on `SELECT <list>` statements without further clauses:
  58,318 of 58,583 statements (99.5%) and 80,130 of 80,426 items (99.6%) parse without fallback.
  The failures are 225 Trino typed literals
  (`TINYINT '1'`), about 8 `{{ op }}` tags in operator position, and negative tests that ClickHouse rejects too.
- Re-casing the recorded keyword slots changes nothing the oracle prints (`mise run corpus:recase`).
  One file stays on the oracle allowlist for the whole-text comparison: `INSERT … VALUES (1); SELECT 1;` on one line,
  the quirk the plan names.
- Tree rules the printer relies on: a node never starts or ends with trivia;
  trivia sits between siblings in the deepest node holding both neighbours,
  so an `{% if %}` and its `{% endif %}` can sit at different depths; each run of `AND` or `OR` is one flat node;
  nesting stops at about 200 levels and degrades to verbatim.
- `mise run corpus` prints parse statistics per statement kind and the top fallback reasons;
  `mise run tree -- FILE` prints a file's tree.
- Known gaps: a `{{ config() }}` before a statement makes the statement verbatim until M5;
  there is no typed layer over the tree.

Review of part 1: a sound base for the printer.
It found the following, all since fixed:

- Wrong keyword slots in four positions, none of which occurs in a corpus: `COLUMNS(…)` where it is the function,
  the arguments of `* EXCEPT` and `APPLY`, `SOME`/`ALL(…)` with other than one argument,
  and the function name in `AggregateFunction(…)`.
- Parsing time on unbalanced nesting: 1 MB of `(` takes about 10 s, and of `interval (` minutes.
- The statistics count statements with designed-opaque regions as parsed.
- The reviewer's own checks found no precedence mismatch in 2,784 cases against `EXPLAIN AST`,
  and the 30-minute fuzz run of the parser found no crash (1.4 million runs).

Part 3: `CREATE TABLE` and `CREATE MATERIALIZED VIEW`.

| Stateless corpus | Statements | Fully parsed | Opaque only in the `SELECT` body | Fallback | Without fallback |
| --- | --- | --- | --- | --- | --- |
| `CREATE TABLE` | 21,524 | 20,426 | 1,035 | 63 | 99.71% |
| `CREATE MATERIALIZED VIEW` | 541 | 0 | 539 | 2 | 99.63% |

- The `SELECT` bodies become trees when part 2 lands.
- Every `CREATE TABLE` and view in the fixtures and the local corpora parses without fallback.
- Re-casing changed 3,203 statements; the oracle judged 3,171 of them and none differs.
- All 514 keyword words, lower case and capitalised, in 16 name positions: no name becomes a keyword slot.
- Remaining fallbacks: MySQL dumps, `settings {{ x }}`, `STATISTICS(trim(BOTH …))`,
  a Jinja tag standing for a modifier or a column, `PARALLEL WITH`, and negative tests.
- Differences from the plan: an unquoted column named `index`,
  `projection` or `constraint` is rejected by ClickHouse and stays a verbatim item;
  clauses may come in any order and repeat, where ClickHouse fixes the order; ClickHouse has no `LIKE` form;
  `AUTO_INCREMENT` is parsed.
- Known gaps: `SETTINGS name` without a value, `INTO OUTFILE` after a `CREATE`, and `ATTACH TABLE` and `REPLACE TABLE`,
  which stay verbatim statements.

Part 2: `SELECT` and `WITH`.
The numbers are for the whole of M3, with the part 1 review fixes in.

| Stateless corpus | Parsed | Opaque by design | Partial | Verbatim | Without fallback |
| --- | --- | --- | --- | --- | --- |
| `SELECT` | 134,928 | 6,496 | 624 | 14 | 99.55% |
| `WITH` | 2,783 | 108 | 24 | 9 | 98.9% |
| `CREATE TABLE` | 21,454 | 3 | 67 | 0 | 99.69% |
| `CREATE MATERIALIZED VIEW` | 536 | 2 | 3 | 0 | 99.45% |

- The plan's targets are 90% for `SELECT` and 95% for `CREATE TABLE`.
  "Opaque by design" is `(EXPLAIN …)` subqueries, `OVER (…)`, window definitions and `INTO OUTFILE` options.
- The DESIGN.md example, the chum fixtures and the local corpora parse every `SELECT`,
  `CREATE TABLE` and materialized view without fallback.
  The dbt models start with `{{ config() }}` and stay verbatim until M5.
- Keyword slots: `tests/keyword_positions.rs` puts every keyword and literal word into every position the grammars know
  (197,493 cases, 7,546 judged by the oracle, none failing); `mise run test:keyword-positions` runs it.
  Over the corpora, re-casing changes 29,183 statements; the oracle judges 28,995 and none differs.
- The oracle sweep judges 270,259 stateless statements the same and none differing;
  verify and idempotence hold on every file.
- Parsing work is bounded: at most 64 looks per token plus 4,096 per statement, past which a statement stays verbatim.
  Real statements need at most 26 looks per token.
  All 67 pathological 1 MB inputs parse in 128 ms or less.
- `--check` over the stateless corpus takes 1.6–1.8 s; per file the median is 98 µs, p95 568 µs, the maximum 38 ms.
- Remaining `SELECT` fallbacks: Trino-dialect tests (228), Jinja (M5), JSON path operators `.^` and `.:` (84),
  pipe syntax `|>` (72), `INSERT` after `WITH` (13).
- Tree shapes the printer must know:
  - A run of binary operators of one priority is one n-ary `BinaryExpr`; `AndExpr` and `OrExpr` are flat runs.
  - `UnionQuery` is a flat run of operands and `SetOperator` nodes, and also wraps a lone query in parentheses.
  - Joins are sibling clauses of `FromClause`; a comma join after a join is a `JoinClause` starting with `,`.
  - `WITH TOTALS` and the like sit in a `GroupByModifier` inside `GroupByClause`.
  - A projection's query has its own entry point: ClickHouse reads `distinct`, `all` and `top` as names there.
- Differences from the plan: joins sit beside `FROM`, not inside it;
  `INTO OUTFILE` parses its keywords and keeps only the file name and options opaque; pipe syntax is not parsed.
