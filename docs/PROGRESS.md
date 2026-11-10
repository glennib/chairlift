# chairlift: implementation progress

Tracks the implementation of milestones M0–M8 from `MILESTONES.md`.
M9 (chum integration) is not part of this run.

## Status

| Milestone | State | Notes |
| --- | --- | --- |
| M0. Repository scaffolding | done | CI green |
| M1. Lossless lexer | implemented | independent review running |
| M2. Verifier, splitter, safe-cleanup CLI | in progress | |
| M3. Parser core | not started | planned in three parts: infrastructure and expressions, `SELECT`, `CREATE TABLE` and materialized views |
| M4. Formatting rules and printer | not started | |
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

## Deviations from the plan

- **Lexer rules (M1)**: where the plan's summary of ClickHouse's lexer differs from ClickHouse's source,
  the source is followed.
  The lexer's token stream is compared with ClickHouse's own lexer, compiled from the fetched source, in the tests.
  - A `.` not followed by a digit is a `Dot`; at offset 0 of the input a `.` always starts a number.
  - `1e` at the end of input is a bare word; `1e+` is the number `1e` and a `Plus`.
  - `$` is a bare word only when a word character follows, otherwise a `DollarSign`.
  - A hex or binary string with a wrong character is an error token that runs to the end of input.
  - ASCII and Unicode whitespace are one `Whitespace` token; ClickHouse emits them separately.
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

Acceptance, as measured by the implementer (the independent review re-runs these):

| Criterion | Result |
| --- | --- |
| Round trip on every corpus file | holds in both modes: 14 fixtures, 12,866 stateless (9 non-UTF-8 skipped), 38 dbt, 9 local |
| Edge-case table of at least 60 | 147 cases (99 SQL, 48 Jinja) |
| Token kinds agree with ClickHouse | identical streams to ClickHouse's compiled lexer on the edge cases and on 12,978 corpus files |
| Throughput of at least 200 MB/s | about 255 MB/s for `tokenize`, 274 MB/s streaming |
| Fuzzing for 10 minutes | 601 s, 1.94 million runs, no crash |
| minijinja differential | about 3,930 tags, no disagreement, nothing allowlisted |

- The keyword list has 514 words from 666 phrases, as the plan expects.
  `mise run keywords:check` verifies the checked-in list against the fetched source.
- `mise run test:corpus` runs the corpus tests;
  the CI corpus job runs it with `CHAIRLIFT_REQUIRE_CORPUS` so that a missing corpus fails.
- Compiling ClickHouse's standalone lexer for the comparison needs clang.
- The private migrations corpus has 4 files and no template, not the 6 the plan lists.
