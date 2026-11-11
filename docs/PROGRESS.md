# chairlift: implementation progress

Tracks the implementation of milestones M0–M8 from `MILESTONES.md`.
M9 (chum integration) is not part of this run.

## Status

| Milestone | State | Notes |
| --- | --- | --- |
| M0. Repository scaffolding | done | CI green |
| M1. Lossless lexer | done | reviewed; CI green |
| M2. Verifier, splitter, safe-cleanup CLI | done | review findings fixed |
| M3. Parser core | done | both grammar reviews' fixes are on `main` |
| M4. Formatting rules and printer | done | acceptance measured; reviews' fixes on `main`; style readings await the project owner |
| M5. Jinja templates | in progress |  |
| M6. CLI polish and performance | implemented, unmerged | complete on the branch `wip/m6-cli`; needs rebasing onto `main` |
| M7. Remaining statement kinds | in progress | part 1 (`INSERT`, one-line statements) done; `ALTER TABLE`, other `CREATE` kinds and access control in progress |
| M8. Hardening and first release | not started | |

## Resuming the work

Work resumed on 2026-11-11.
This section says what is unmerged and what comes next.

| Branch | Holds | State |
| --- | --- | --- |
| `main` | M0–M4 | the tip passes `mise run ci` with the corpora present |
| `wip/m6-cli` | M6 | complete, every commit passed `mise run ci`; based on an older `main` |

Next:

1. Rebase `wip/m6-cli` onto `main`.
   The CLI's diagnostics need an arm for `Error::Template`, whose `TemplateError` carries a `reason`,
   and the message of `Error::Layout`; snapshots that quote formatted output need refreshing.
2. Measure a 1 MB file through the binary on a quiet machine (target 100 ms).
   Under load it took 0.12–0.19 s; in process, on a quiet machine, 62 ms before the layout fixes.
3. M5 and the remaining M7 parts (`ALTER TABLE` with the other `CREATE` kinds; access control) are in progress.
4. M8 last.

Only the tip of a branch is checked with `mise run ci` from here on, not every commit.

### Setup

- `mise install`, then `mise run corpus:fetch`; set `CHAIRLIFT_CORPUS_DIR` to share one copy between checkouts.
- `CHAIRLIFT_LOCAL_CORPORA` takes the local-only corpora, `:`-separated.
  They are never vendored.
- `CHAIRLIFT_REQUIRE_CORPUS=1` makes a missing corpus fail instead of skip.
- The comparison with ClickHouse's own lexer needs `clang`.
- Tasks: `mise run ci`, `test:corpus`, `test:keyword-positions`, `test:templates`, `corpus`, `corpus:oracle`,
  `corpus:recase`, `corpus:expressions`, `tree -- FILE`, `fuzz:lexer`, `fuzz:parser`, `fuzz:format`, `fuzz:doc`.

### Notes for M5, M7 and M8

M5 (Jinja templates), in addition to its scope in `MILESTONES.md`:

- The verifier models one render, with every `{% if %}` branch taken.
  The template defects found so far were invisible to it.
  `mise run test:templates` renders generated templates for every branch combination and loop counts
  and compares the renders, including the text of `::` literals;
  it fails when any of the three template defects is put back.
  Extend it rather than start again, and plug rendered variants into the oracle at `oracle::variants`.
- A file where rendering can remove the line break that ends a line comment is left unchanged and reported
  (`Error::Template`, exit 2),
  unless rendering resumes right after a block tag or Jinja comment that ends its line without a marker.
  Two stateless templates are refused: a comment ending a macro body before `{%- endmacro %}`.
- An implicit table alias must not cross a block tag.
  With alternative `SELECT` branches under `{% if %}`/`{% else %}`,
  the first branch's table takes the next `select` as its alias, which is how ClickHouse reads the unrendered text.
- A `{{ config() }}` before a statement makes the whole statement verbatim; all dbt models start that way.
- In a template with block tags, everything from an `INSERT … FORMAT` to the end of the file is left as it is.
  The rule is broader than it needs to be.

M7: the grammar and layout guides are the rustdoc at the top of `src/syntax/grammar/mod.rs` and `src/layout/mod.rs`.
Add every new keyword position to `tests/keyword_positions.rs`.

M8: no release has been made and no release workflow runs.
`dist generate --mode=ci` creates the cargo-dist workflow, and `release-plz.yml` runs on manual dispatch only.

### How the work was checked

- An implementer's report was never taken as acceptance.
  A second agent re-measured each milestone and attacked it with generated input, and found defects every time:
  in the lexer's error handling, the verifier, keyword slots, parsing time, template rendering,
  and the layout fixes themselves.
- The checks that found the most: comparing with ClickHouse's own lexer and `EXPLAIN AST`,
  the generated keyword-position test, rendering templates for every branch combination,
  and fuzz runs of 30 minutes rather than a few.
- A long-running check should signal its end through its exit.
  Waiting on `pgrep -f` matches the waiting shell itself and never ends,
  and a `grep` that skips binary files never finds a sentinel in a log holding non-UTF-8 bytes.

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
  - M7 part 1: 9 tree snapshots under `src/syntax/grammar/{insert,simple}/snapshots/`
    and `tests/snapshots/format__statements__*.snap`.
  - M6, unmerged: new and changed snapshots in `tests/cli.rs` and `src/bin/chairlift.rs`.
  - M4 part 3: five of the `tests/snapshots/format__*.snap` files changed:
    the chum migrations lose column alignment and get `(` on its own line, and the design example is re-cased.
  - M4 part 2: 14 formatted-fixture snapshots, `tests/snapshots/format__*.snap`.
  - M4 part 1: the inline snapshot in `doc::tests::a_document_dumps_as_indented_nodes`.
  - M3 part 2: 11 tree-dump snapshots under `src/syntax/grammar/select/snapshots/`.
  - M3 part 3: 10 tree-dump snapshots under `src/syntax/grammar/create/snapshots/`.
  - M3 part 1: 24 tree-dump snapshots under `src/syntax/snapshots/`
    and `src/syntax/grammar/{exprs,types,common,fallback}/snapshots/`.

## Style questions for the project owner

Readings chosen where the plan and the layout proposal leave a rule open.
None blocks the work.

- The proposal's section 2b fills literal lists and its section 3 keeps `true` and `NULL` as written.
  `MILESTONES.md` says one item per line and fixed casing, and it wins.
- A `;` that has to go on its own line after a trailing line comment is printed at the statement's column.
- "A statement spans several lines" (the blank-line rule) is read as "its layout broke".
- A lambda body that does not fit goes one indent in from the line the lambda starts on.
  `CASE … END` is placed relative to the line it starts on in the same way.
- An `OR` group inside parentheses hangs: the first operand on the `(` line, the `OR` lines one level deeper.
- A list clause with one item keeps the item after its keyword and breaks inside it.
  A single `SELECT` item that does not fit moves to the next line.
- The operator stays on its line before `IN (list)` and before a subquery.
- A statement kept as written starts at column 0, and a comment trailing it keeps its spacing.
- `;;` stays on the line of the statement before it.
- `INSERT` and the one-line statements:
  - `INSERT` header clauses break to column 0; after a column list that broke, `VALUES` stays on the `)` line.
  - The query after `INSERT` always starts a line at column 0.
  - `VALUES` rows keep their line breaks and the first row's indentation as written.
  - A one-line statement whose list does not fit puts one item per line, one level in,
    and what follows the list starts a line at column 0.
- `CREATE TABLE`, on `main`:
  - A column definition that does not fit breaks before each modifier
    (`DEFAULT`, `CODEC`, `TTL`, `COMMENT`), one level in; `NULL` and `COLLATE` stay with the type.
    The proposal keeps `COMMENT` at the end of the column line, which the printer cannot measure around.
  - `AS` before a query: after a header that ends in the name, a `TO` target or a column list, `AS` ends that line,
    as in the proposal's materialized view.
    After an `ENGINE` or storage clause, `AS` goes on a line of its own,
    because `ORDER BY tuple() AS` reads as an alias.
    `AS (SELECT …)` hugs its bracket.
    In both cases the query starts at column 0.
  - `REFRESH`, `TO`, `EMPTY`/`CLONE`/`POPULATE` and `AS` a table stay on the header line,
    and break one level in when it does not fit.
  - The column list is always one element per line, even for one short column.
  - `CODEC(…)` is glued; `DEFINER = u@'%'` has no spaces around `@`.

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
- **Whitespace next to a Jinja `-` marker (M4)** is re-indented like any other whitespace.
  The plan's style decision treats it as fixed.
  The review of the layout recommends against freezing it:
  the renderer strips the whole run next to a marker in every render, and in 120,000 generated templates,
  each rendered in 12 contexts, the whitespace that changed a meaning was never the whitespace next to a marker.
  Freezing would also leave `AND …` after a `-%}` at column 0 in the proposal's own section 2g example.
  The exception: a line comment followed only by whitespace and a left-marker tag has its end decided by the render,
  so such a file is left unchanged and reported (on `wip/layout-fixes`).
  **For the project owner to confirm.**
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

Review of the `SELECT` and `CREATE` grammars: no blockers; every reported number reproduced.
Fixes in progress for:

- Six classes of wrong keyword slot, each needing a rare name in one position and none occurring in a corpus:
  `(with - 1)` with a column named `with`, a table alias `select`, the `ELSE` branch of `viewIfPermitted(…)`,
  `from(` read as a `FROM`-first query, `CREATE TABLE t AS with(1)`,
  and a setting or TTL key named like a `CREATE` clause word.
- `FORMAT`, `INTO OUTFILE` and a `SETTINGS` after `FORMAT` belong to the whole union
  or `CREATE` statement in ClickHouse, but sit inside the last `SELECT` in the tree.
- 385 of the 778 keyword-slot positions in the stateless corpus appear only in upper case,
  so re-casing the corpus never exercises them; the generated position test is being extended to cover them.

### M4

Part 1: the document IR and printer, in `chairlift::doc`, independent of the syntax tree.

- 35 hand-built documents reproduce the layout proposal's examples byte for byte, the DESIGN.md example included,
  and every output passes the verifier.
- The printer measures each group once.
  It agrees with a reference printer that decides each group by printing it on 5 million random documents.
- Property tests over random documents and random SQL: no trailing whitespace, every token printed once,
  the output verifies, re-printing changes nothing.
- Two tokens touch only where `can_touch` allows it: `-` and `-1` print as `- -1`.
- Building, checking and printing a 1.1 MB document takes about 15 ms.
- Known gaps: a trailing comment is not measured, so a line with one can exceed the width; width counts characters,
  not East Asian display width; there is no fill layout, which the plan rejects.

Part 2: the layout framework, expression layout and `SELECT`/`WITH`, wired into `format`.

- Every `SELECT` and `WITH` example of the layout proposal comes out byte for byte,
  from flattened input and from the expected text.
  The `CREATE` examples and the DESIGN.md example wait for part 3;
  the query of the DESIGN.md example already formats as the proposal's section 4.
- A statement the layout has no rule for, or cannot lay out safely, is printed as written.
  Stateless corpus: 141,184 of 142,041 `SELECT` statements are formatted (99.4%).
  Kept as written: 708 where the rendered template joins tokens, 135 with a tag hidden in a comment or string,
  14 not parsed.
- Verify, idempotence and the exactly-once token check hold on every corpus file.
  The oracle judges 270,259 stateless statements the same and none differing.
- `--check` over the stateless corpus takes 1.9 s single-threaded; per file the median is 95 µs and p95 538 µs.
  A 1.1 MB file formats in about 88 ms: split 7.5, parse 30, layout 24, token check 1.5, print 5.8, verify 13.5.
- Lines over 100 characters in the formatted stateless corpus: 34,500,
  of which 29,100 are in statements kept as written, 2,000 carry a trailing comment, 1,600 are in opaque regions,
  1,500 hold one long token.
  A sample of the remaining 251 showed no missed break.
- M2's `cleanup()` is gone; a statement kept as written prints through the document builder.
- Known gaps: a long chain of `+` breaks one operator at a time from the end;
  an alias after an expression that broke can pass the width; trailing comments are not measured.

Part 3: `CREATE TABLE` and `CREATE MATERIALIZED VIEW` layout.

- All layout goldens pass byte for byte from flattened input and from the expected text: the seven `CREATE` examples,
  the materialized view, and the DESIGN.md example.
- Stateless corpus: 21,366 of 21,420 `CREATE TABLE` statements (99.7%) and all 541 materialized views are formatted.
  Verify, idempotence and the token check hold on every file, also at widths 10, 30 and 60.
  The oracle finds no differing statement in any corpus.
- 1 MB of `CREATE` statements formats in about 116 ms, linear to 4 MB.
- Known gaps: `AS (SELECT …)` puts `(` alone on the line after `AS`;
  `REFERENCES db.p (b)` keeps a space before the bracket; a `CREATE` header has no break point.

Review of part 2 (the layout framework and `SELECT`): safe on plain SQL,
where the verifier turns the faults found into errors that leave the file alone.
Fixes are on `wip/layout-fixes` for:

- **A line comment before a left `-` marker**
  (`{%-`, `{{-`, `{#-`):
  the layout changed how much the comment swallows in renders where a branch is not taken or a loop runs more than once.
- **`[{{ x }},2]::String`** gained a space after the comma; `::` reads the literal's source text.
- Non-idempotence around a comment before a comma, which caused nearly every case the generators found.
- `INSERT … FORMAT` data that ends before a `)` failed verification, a regression from M2.
- An operator dropped to a new line after an operand that broke
  (646 instances in 200 stateless files);
  a multi-line string broke its operator group; an `OR` group in brackets as the first condition was not indented;
  an own-line comment after a clause keyword lost its indentation; `LIMIT … BY` lists did not break.
- A quadratic loop over verbatim regions; JSON path operators printed with a space; blank lines around comments.

The fuzz run of `format` for 30 minutes on part 2 found no crash (339,000 runs).

### M6

Implemented on `wip/m6-cli`; see "Resuming the work" for what is verified.

- `miette` diagnostics with `--quiet`, `--verbose`, `--color`; parallel processing with `-j/--jobs`,
  output in path order; `--stdin-filename`; `--verify`.
- `--verify` over the stateless corpus with 8 jobs: 268,929 statements judged, none differing, in 290 s.
  Files that formatting leaves unchanged are not judged.
  Templates are judged unrendered.
- `--check` over the stateless corpus: 2.3 s with one job, 0.7 s wall and 4.4 s CPU with eight.
- A 1.1 MB file formats in 62 ms in process: read 0.3, split 6, parse 20, layout 19–21, token check 1.5, print 4.7,
  verify 11, dropping the tree 4.4.
- A faster oracle route through `clickhouse local` was tried and rejected: it drops `INSERT` data.

### M4, closed

The layout fixes and the fixes for the second grammar review are on `main`.
Only the tip was checked with `mise run ci`; 43 of the 49 commits had also passed on their own.

Review of the `CREATE` layout and the layout fixes found, and the fixes cover:

- **One more shape of the template comment defect**: a `{{ v -}}` at the end of a line comment strips its line break.
  One rule now covers the class (see the notes for M5).
- Ordinary dbt templates were refused: a `--` line followed by `{%- if %}`.
  They format now.
- The first operator fix made some lines longer than before it.
  An operator now stays on a closing bracket's line only if that line starts with the closer,
  nothing after it can break, and the operator and the start of its operand fit.
- The template property test could not see a change inside a `::` literal.
  It compares that text now.
- A comment before `)` went to column 0; `BETWEEN` had no break point; a cast's type broke at narrow widths;
  a type nested more than four deep is kept on one line; `TTL … DELETE WHERE` breaks before `WHERE`.

Acceptance, measured at the tip:

| Criterion | Result |
| --- | --- |
| DESIGN.md example byte for byte | yes, from flattened input and from the expected text |
| Proposal examples for M3 statements | all 36 give the recommended alternative |
| Verify, idempotence, every comment and tag once | 0 failures over 12,936 files |
| Oracle, local corpora | 9 of 9 files and 32 of 32 statements unchanged |
| Oracle, stateless: at most 50 differing files | 0; 270,250 statements judged, 125,647 of them changed by formatting |
| `--check` over the stateless corpus under 15 s | 3.3 s single-threaded on a loaded machine |
| Per-file p95 under 5 ms | 2.43 ms |

- Stateless corpus: 21,390 of 21,420 `CREATE TABLE` statements,
  all 541 materialized views and 141,755 of 142,017 `SELECT` statements are formatted; 2 templates are refused.
- The reviewer's generators at the tip: 220,000 templates and 120,000 SQL texts with no render difference,
  nothing non-idempotent and no verify failure.
- Fuzz, 3 minutes each: `format` 136,000 runs, `parser` 64,000 runs, no crash.
- A 1.05 MB file formats in 0.12–0.19 s on a loaded machine, against 0.11–0.13 s before the fixes.

### M7

Part 1: `INSERT` and the one-line statements.
Only the tip was checked with `mise run ci`.

| Stateless corpus | Statements | Parsed | Formatted |
| --- | --- | --- | --- |
| `INSERT` | 25,314 | 25,253 | 24,867 |
| `DROP` and its kin | 36,459 | 36,457 | 36,290 |
| `SET` | 20,449 | 20,424 | 20,143 |
| `OPTIMIZE` | 1,598 | 1,591 | 1,594 |
| `ATTACH` | 635 | 561 | 632 |

- `DETACH`, `TRUNCATE`, `USE`, `RENAME`, `EXCHANGE` and `UNDROP` all parse.
  The statements that do not parse are ones ClickHouse rejects too.
- `INSERT` data is one token, kept byte for byte: the verifier compares it,
  and a corpus test requires the same data tokens before and after.
  The oracle cannot judge data in formats other than `VALUES`.
  After `VALUES` rows the rest of the line stays as written,
  because `clickhouse-format` reads rows to the end of the line.
- An `ATTACH` with a full definition has its head parsed and the definition kept as written.
- Verify and idempotence hold on every corpus file.
  The oracle judges 270,251 statements and none differs; 148,451 statements are changed by formatting.
- Keyword positions: 617,679 cases with no failure.
  One wrong slot was found and fixed: `true.*` in an `INSERT` column list is a table name.
- Statements still verbatim at statement level on the stateless corpus: 22,492 of 275,515 (8.2%, from 39%).
  By leading keyword: `ALTER` 4,991, `SYSTEM` 3,887, other `CREATE` kinds 3,774, `EXPLAIN` 2,941, `PRINT` 1,454,
  `SHOW` 1,189, `DESCRIBE` 1,059, `DROP` of access entities 727, `UPDATE` 285, `DELETE` 263.
- Where ClickHouse differs from the plan: `INSERT` takes no `SETTINGS` after `FORMAT` and has no `WATCH` form;
  the `SETTINGS` of an `INSERT … SELECT` belongs to the `SELECT`; `DETACH` and `TRUNCATE` take one name.
