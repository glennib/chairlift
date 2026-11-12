# chairlift: implementation milestones

Status: plan, 2026-11-10.
This document refines `DESIGN.md` into a sequence of milestones that an agent
or an orchestrator can implement one at a time.
It records the decisions taken on 2026-11-10, the evidence behind them
(the spike reports under `docs/spikes/`),
and for each milestone the scope, the deliverables and the acceptance criteria.
`DESIGN.md` stays as the background; where the two disagree, this document wins.

How to use it:

- Implement milestones in order; each one builds and passes its own tests before the next starts.
- A milestone's acceptance criteria are the definition of done.
  Numbers in them come from the spikes and are targets, not guesses; if one proves wrong, change it in this document.
- Open questions listed under a milestone are decided by the project owner before or during that milestone,
  not by the implementer.
- The spike code was discarded.
  The reports describe what worked, with code-level detail where it matters; an implementer re-derives the code.

## Decisions

All decided on 2026-11-10 by the project owner unless marked *(derived)*, which means it follows from a spike result.

### Architecture

- **Hand-written Rust pipeline**: lossless lexer, tolerant parser emitting an event list,
  a `rowan` concrete syntax tree built from the events, a Wadler-style document IR with a printer of our own,
  and a token-stream verifier.
  Tree-sitter plus Topiary was spiked and rejected: tree-sitter parsed well enough,
  but Topiary has no width-aware layout, glues tokens together wherever a query is missing,
  and either refuses or corrupts files with parse errors (`docs/spikes/2026-11-10-tree-sitter-topiary.md`).
  A tree-sitter grammar remains a possible side project for editor highlighting; it is not on this plan.
- **Printer**: our own, about 200 lines, with the IR shaped so the `pretty` crate could replace it.
  `biome_formatter` is rejected: its last crates.io release is from 2024-03,
  it needs eight crates pinned to `=0.5.7` and pulls 105 dependencies for features
  that take 200 lines to write *(derived)*.
- **Lexer**: hand-written, not `logos`.
  The hard cases (Jinja tags with their own strings, tags glued into words, `$tag$` heredocs, nested block comments,
  `.` versus `.5`, `#` only before a space or `!`, Unicode whitespace) need context,
  and the spike's hand-written lexer runs at 300–950 MB/s *(derived)*.
- **Jinja tags are found by chairlift's own lexer**, not by minijinja's. minijinja's tokenizer is built for rendering:
  it drops comments, raw delimiters and stripped whitespace, stops at the first error,
  and lives behind an unstable feature that chum pins to `~3.1.0` *(derived)*. minijinja is a dev-dependency used
  as an oracle: a differential test checks both find the same tag spans.
- **Own crate in this repository.**
  A workspace with chum can come later; release-plz then switches both to per-package tags (`chum-v0.6.0`),
  which cargo-dist's trigger already matches (`docs/spikes/2026-11-10-prior-art-addendum.md`).
- **Only whitespace and keyword case change.**
  Never add, remove, reorder or rewrite tokens.

### Style

- **Keywords UPPER**, re-cased only in keyword slots (see "Keyword classification" below).
  Function, type, engine, setting, codec and layout names, identifiers and aliases keep their case.
- **Literals**: `true`, `false`, `inf`, `nan` are written lowercase and `NULL` uppercase, matching `clickhouse-format`.
- **Line width 100**, configurable with `--line-width`.
- **`chl.toml`** (decided 2026-11-12, after M8; where it is looked for amended the same day):
  the nearest one holds the settings the flags give and nothing else: `line-width`, `jinja` and `exclude`.
  It is looked for, as black looks for its configuration, in and above the deepest directory that holds all the paths,
  or for stdin the directory of the `--stdin-filename` name, or else the current directory; one file per run, not merged.
  A flag given on the command line wins over it; its excludes are relative to its directory and add to `--exclude`.
  The style stays one style: the file adds no setting the flags do not have.
- **Lists**: a list stays on one line if it fits, otherwise one item per line.
  No filling of literal lists (`IN (…)`, arrays): they go one per line too.
  A SELECT list with more than one item always splits, even if it would fit.
  GROUP BY, ORDER BY, SETTINGS, tuples, arguments and column lists follow the fit rule.
- **Conditions**: hanging style with leading operators.
  The first condition stays on the `WHERE` line; continuation lines start with `AND`/`OR` at one indent.
  The same for `ON`, `PREWHERE`, `HAVING`, and for `OR` groups inside parentheses, one level deeper.
  The `AND` inside `BETWEEN … AND …` is not a break point.
- **Trailing commas.**
  A comment can pin a comma where the author put it
  (tokens are never reordered across a comment), so a leading comma survives in that one place.
- **No column alignment** in `CREATE TABLE`: one space between name, type and modifiers.
- **Blank lines**: between statements keep 0, 1 or 2 as written
  (more collapse to 2), but force one before and after any statement that spans several lines.
  Inside a statement keep at most one, and only between list items or before a clause;
  none right after `(` or before `)`.
  A header comment keeps the 0 or 1 blank line the author gave it.
  The file starts without a blank line and ends with exactly one newline.
- **Comments**: an own-line comment attaches to the next node and takes its indent;
  a trailing comment stays on its line, two spaces after the code; a line comment forces its enclosing group to break;
  a `/* */` comment inside a line is an atom; comment text is never reflowed,
  and a multi-line `/* */` comment is kept byte for byte.
  A `--` comment line is a token, so a block of them is re-indented line by line, keeping relative indent.
- **Jinja**: `{% %}` block tags are stepped over by the parser like comments,
  so it sees the SQL of every branch at once.
  A block tag goes on its own line at the indent of the SQL it wraps; the wrapped SQL is not indented further;
  `{% else %}`/`{% elif %}`/`{% endif %}`/`{% endfor %}` align with their opener;
  a tag between statements goes to column 0.
  `{{ … }}` is one opaque token wherever an expression or name can stand; `events_{{ suffix }}` is one word;
  a tag inside a string literal is part of the string; nothing inside any tag is changed, including `{%-`, `-%}`,
  `{%+` and `+%}` markers.
  Whitespace adjacent to a `-` marker is treated as fixed (not moved), because the render strips it.
  Structural understanding of `if`/`for` blocks is a later milestone, taken only if real files need it.
- **Parentheses**: a subquery, a single CTE and a `PROJECTION` body hug their `(`
  (open at the end of the line, close at that line's indent).
  Several CTEs go one per line, indented under `WITH`.
  The `CREATE TABLE` column list keeps `(` alone on its own line, as `SHOW CREATE TABLE` prints it.
- **Statement shapes**: `CREATE MATERIALIZED VIEW … TO … AS` ends the header line and `SELECT` starts at column 0;
  `INSERT INTO t (cols)` on one line, `SELECT` at column 0; `ALTER TABLE` commands are a list
  (one stays on the `ALTER` line);
  `GRANT` breaks before `ON`, `TO`, `WITH GRANT OPTION` when it doesn't fit;
  `CASE` expands with `WHEN`/`ELSE` one level in and `END` at `CASE`'s indent;
  a lambda is one argument and breaks after `->` when its body doesn't fit;
  `;` goes at the end of the statement's last line (own line only after a trailing line comment).
- The full rendered proposal with alternatives is `docs/spikes/2026-11-10-layout-proposal.md`;
  its section 3 lists the ClickHouse-specific clauses and their layout.

### Verification

- **The token check runs on every format**, including `--check` and `--diff`, and has no off switch:
  it costs about 80 ns per token *(derived)*.
  It compares token kinds and text; a bare word may differ in case only at positions the formatter recorded
  as re-cased keywords; Jinja tag text and comment text must match byte for byte.
- **Idempotence** (`fmt(fmt(x)) == fmt(x)`) is checked in tests and by the corpus tool, not on every run.
- **The ClickHouse oracle** (`clickhouse-format -n --oneline --show_secrets`) runs in tests and in the corpus tool.
  The binary does not run it, so chairlift needs no ClickHouse installation to run.
  The binary's `--verify`, built in M6, was removed before the first release:
  it judged neither Jinja nor what `clickhouse-format` cannot parse,
  and ran whatever `clickhouse-format` version the user had, without the allowlist of its quirks.
  `--show_secrets` is required: without it engine arguments and passwords print as `[HIDDEN]` *(derived)*.
- **Every comment and tag is emitted exactly once**: an assertion in the printer, as Prettier does.
- chum's migration checksum is SHA-384 over the source text,
  so reformatting an applied migration registers as drift and chum has no override.
  **Out of scope here**; recorded as a risk for the chum integration milestone.

### Scope

- Spike code is not kept; reports live under `docs/spikes/`.
- Out of scope, as in `DESIGN.md`: lint rules, other dialects, semantic checks, formatting inside Jinja tags,
  reflowing comment prose, a language server.

## Architecture notes for implementers

These are the rules the spikes found necessary.
They apply across milestones.

### Lexer

- Every byte becomes a token; whitespace and comments are tokens; tokens concatenate back to the input.
- Follow ClickHouse's lexer rules exactly;
  the table in `docs/spikes/2026-11-10-clickhouse-internals-and-corpus.md` is the checklist.
  Notable: `#` is a comment only before a space or `!`; `/* */` nests;
  a leading `.` is a number unless the previous significant token is `)`, `]`, a word, a quoted identifier or a number;
  `-1` is two tokens; `_` is allowed between digits; `$tag$` heredocs need an exact closer or they are a bare word;
  `{name:Type}` is five tokens and the parser handles it; Unicode whitespace and the BOM count as whitespace.
- Jinja is lexed first: `{{ … }}`, `{% … %}` and `{# … #}` are single tokens, with Jinja-aware scanning inside
  (strings, balanced brackets) so `{{ "%}" }}` ends in the right place; `{% raw %}…{% endraw %}` is opaque.
  A tag touching word characters joins the word into one token.
  Inside a SQL string, tags are part of the string.
- Lexer error tokens (unclosed quote or comment, lone `!`, unknown byte) mean the token identity of the rest of the file
  cannot be guaranteed: the file is left unchanged and reported.
- Input must be valid UTF-8; a non-UTF-8 file is reported and left unchanged.

### Keyword classification

ClickHouse has no reserved words for identifiers: `key`, `value`, `date`, `order`, `limit`, `settings`,
`from` are all valid column names, and `Null` is an engine.
Re-case a bare word only when all three hold:

1. it is a bare word (not quoted, not inside a verbatim or Jinja region);
2. its uppercase form is one of ClickHouse's 514 keyword words
   (`APPLY_FOR_PARSER_KEYWORDS` in `src/Parsers/CommonParsers.h`; regenerate the list per ClickHouse release);
3. the parser consumed it in a keyword slot: a statement or clause keyword, an operator word
   (`AND OR NOT IN LIKE IS BETWEEN AS ON USING CASE WHEN THEN ELSE END INTERVAL DISTINCT …`),
   a column-definition keyword (`DEFAULT MATERIALIZED ALIAS CODEC TTL COMMENT …`), or a literal keyword.

Positional rules the spike needed: `ENGINE =` takes a name, not an expression;
`SETTINGS` opens a clause only when followed by `name =`;
an implicit alias cannot be one of the 44 words in `ParserAlias::restricted_keywords`.
The token check trusts this classification, so the oracle is the only thing that catches a mistake here.
This is the main source of residual oracle differences and the reason the oracle runs over the whole corpus in CI.

### Parser and tree

- The parser writes start/token/finish events to a list; a checkpoint is the event count plus the token position;
  rolling back truncates the list.
  The `rowan` tree is built from the events at the end, because rowan's builder cannot roll back.
- **Fallback ladder**, smallest first.
  A failed parse never fails the file; it widens the verbatim region:
  1. **List item** (column definition, select item, argument, tuple or array element, setting, FROM table):
     if the parsed item does not end at `,`, the list end, `;` or end of input,
     it becomes verbatim up to the next `,` or closing bracket at the same depth.
  2. **Clause**: if a clause is not followed by another clause keyword or the statement end,
     the whole clause is verbatim up to the next clause keyword at depth 0.
     Clauses may come in any order and repeat;
     that is what lets `{% if %} WHERE a {% else %} WHERE b {% endif %}` parse.
  3. **Statement**: anything the parser does not know is verbatim up to the next top-level `;`.
- Verbatim regions keep their bytes, including keyword case and internal whitespace; at most the block is re-indented.
- **Whitespace that ClickHouse reads as text** must stay verbatim even inside parsed constructs:
  the left operand of `::` (`[3,4,5]::Array(Int64)` reaches the server as the string `[3,4,5]`),
  and inline data after `INSERT … VALUES` or `INSERT … FORMAT X`.
- **Statement splitting** copies clickhouse-client's rule, not `clickhouse-format`'s: after `VALUES`,
  data runs to the terminating `;` (rows are parsed as parenthesised tuples); after any other `FORMAT name`,
  data runs to the first blank line or end of input.
  A `;` inside a string, quoted identifier, heredoc, comment or Jinja tag never splits.

### Comments and blank lines

Comments and Jinja block tags stay in the tree between siblings; there is no separate attachment pass.
The number of newlines before a comment decides its placement: none means trailing
(stays on its line after the comma), one means own line before the next item, two or more keeps one blank line.
This is handled between statements, clauses, list items and `AND`/`OR` operands;
elsewhere the enclosing item or clause falls back to verbatim.
A later refinement attaches comments per list so that a comment inside call arguments no longer forces verbatim.

### Printer

- Wadler-style: text, space, soft line, hard line, group, indent, with closers aligned to their opener.
  A group containing a line comment or a block tag can never print flat.
- Layout spaces are emitted lazily and token text is never trimmed.
- Two tokens may touch only if their concatenation lexes back to the same two tokens; otherwise a space is emitted.
  This is what stops `-` `-1` from becoming a comment and `#` from losing its space.
- Build the document per node with cached "contains comment", first and last token;
  the spike's rescanning made document building the slowest stage (28 ms of 50 ms on 1 MB).

## Corpora and tooling

Every milestone from M1 on is tested against these.

| Corpus | Where | What it is for |
| --- | --- | --- |
| DESIGN.md example | `DESIGN.md`, section "Layout: a starting point" | The golden fixture; the proposal's output in `docs/spikes/2026-11-10-layout-proposal.md` section 4 |
| chum fixtures | `~/devel/glennib/chum/tests/{migrations,template_migrations,bootstrap_migrations,upgrade_migrations,question_mark_migrations}` | DDL, dictionaries, templates; chum is MIT/Apache so these can be vendored into `tests/fixtures/` |
| ClickHouse workshop | `~/devel/amedia/clickhouse-workshop` (5 files) | Keyword-named columns, trailing comments after commas, aligned columns |
| Private migrations | `~/devel/amedia/tally/crates/clickhouse/migrations` (6 files, one `.up.j2.sql`) | Run locally only; never vendored |
| ClickHouse stateless tests | `tests/queries/0_stateless` at tag `v26.10.2.103-stable`: 12,619 `.sql` (9 not UTF-8), 241 `.sql.j2` | Breadth: 275k statements, 95.5% accepted by the oracle |
| ClickHouse docs | `docs/reference/**/*.mdx` at the same tag: 6,955 ```` ```sql ```` blocks, 62.7% accepted | Optional extra breadth, filtered through the oracle first |
| dbt-clickhouse samples | public repos, see `docs/spikes/2026-11-10-prior-art.md` section 6 | `{{ config() }}`, `{{ ref() }}`, `{% if is_incremental() %}`, `{% set %}` holding DDL in strings |

Fetching the ClickHouse corpus (sparse, blobless, shallow; cache it in CI):

```shell
git clone --depth 1 --filter=blob:none --sparse --branch v26.10.2.103-stable \
    https://github.com/ClickHouse/ClickHouse corpus/ClickHouse
git -C corpus/ClickHouse sparse-checkout set tests/queries/0_stateless
```

The oracle is ClickHouse 26.10 (`mise use clickhouse@26.10.2.103-stable`;
the binary provides `clickhouse-format` and `clickhouse local`).
Rules for using it:

- Call it as `clickhouse-format -n --oneline --show_secrets`; compare the canonical text of input and output.
- One process costs about 72 ms of startup and 10 ms of work per file;
  batch files per process and fall back to per-file runs when a batch fails,
  so one bad statement does not hide the rest.
- Classify its errors: a failure at `{` or `%`, or mentioning `substitution name`, means unrendered Jinja;
  `Can't format INSERT query`, `Empty query`, or the input itself failing means the oracle cannot judge the file.
  Only "input parses, output doesn't" or "canonical forms differ" is a chairlift bug.
- Multi-line `INSERT … VALUES` is cut at the first newline by `clickhouse-format`;
  the harness joins the rows onto one line before calling it, for both input and output.
- Templates are rendered with minijinja before the oracle sees them,
  once per branch combination of their `{% if %}` blocks, with chum's environment settings
  (default delimiters, `trim_blocks` and `lstrip_blocks` off, `keep_trailing_newline` on, strict undefined)
  and stub `sql_*` filters.
- Residual differences that are oracle quirks, not chairlift changes, get a documented allowlist:
  the `::` left-operand text, and `INSERT … VALUES (…); SELECT 1;` on one line.

The whole stateless corpus through chairlift's check takes about 14 s single-threaded in the spike;
through the oracle about 3 minutes on 10 cores.
CI runs chairlift's own checks on every push and the oracle sweep on a schedule or on demand.

## Milestones

Effort figures are focused agent-days from the spike's estimate (19–30 in total) and are rough.

### M0. Repository scaffolding

Goal: a crate that builds, lints, tests and releases the same way chum does, with the corpus and oracle wired in.

Scope:

- Library crate `chairlift` with a `cli` feature and a `chl` binary, mirroring chum's layout
  (`src/lib.rs`, `src/bin/`), `clippy::pedantic` warnings, `rustfmt.toml`, MIT/Apache dual licence.
- Dev tooling: `cargo nextest`, `insta` for snapshots, `criterion` or `divan` for benchmarks,
  `cargo-fuzz` targets directory (fuzzing itself comes in M1), `mise.toml` pinning ClickHouse 26.10.
- CI: build, clippy, tests, formatting; a cached checkout of the stateless corpus;
  an `oracle` job that is manual or scheduled.
- Release: release-plz plus cargo-dist as in chum, producing a static binary.
- A `README.md` with the one-paragraph pitch and the style summary; `DESIGN.md` and this document linked.

Deliverables: the crate skeleton, CI green on an empty library,
a `tests/fixtures/` directory with the DESIGN.md example and chum's fixtures vendored,
a `corpus/` directory ignored by git with a fetch script.

Acceptance:

- `cargo nextest run`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass in CI.
- `mise run ci` (or the task runner chum uses) runs everything above locally.
- The corpus fetch script populates `corpus/ClickHouse/tests/queries/0_stateless` and CI caches it.

Effort: 1 day.

### M1. Lossless lexer for SQL and Jinja

Goal: every byte of every corpus file becomes a token, and the tokens concatenate back to the input.

Scope:

- Token kinds mirroring ClickHouse's `TokenType` list plus `Whitespace`, `LineComment`, `BlockComment`, `JinjaExpr`,
  `JinjaBlock`, `JinjaComment`, `Verbatim`, and error kinds.
- The lexer rules from the ClickHouse checklist, Jinja scanning as described under "Architecture notes",
  and keyword classification of bare words into an enum
  (the 514-word list generated from ClickHouse's source by a script checked into `xtask/` or `scripts/`).
- Jinja mode on or off (the CLI sets it by extension; in a plain `.sql` file `{{` is two braces).
- Property tests: round trip (`concat(tokens) == input`), no panics, and every token non-empty.
- A differential test against minijinja's tokenizer for tag spans over the template corpora.
- A fuzz target for the lexer.
- A benchmark over the concatenated stateless corpus.

Not in scope: statement splitting (M2), any parsing.

Acceptance:

- Round trip holds on every file of every corpus, including the 241 `.sql.j2` stateless files and the dbt samples.
- Token kinds agree with ClickHouse's lexer on a hand-written table of at least 60 edge cases taken from the checklist
  (`#x` is an error, `1--1` is number plus comment, `.5` after `)` is a dot, `0x1.8p3` is a number,
  `$tag$a $$ b$tag$` is one heredoc, `‘str’` is a string, `events_{{ s }}` is one word, `'{{ x }}'` is a string).
- Throughput of at least 200 MB/s on the concatenated stateless corpus in release mode (the spike measured 312).
- Fuzzing for 10 minutes finds no panic.

Effort: 1–2 days.

### M2. Verifier, statement splitter and safe-cleanup CLI

Goal: a binary that can already run in pre-commit and format-on-save, changing only what is risk-free,
with the safety net that every later milestone relies on.

Scope:

- Statement splitter on top of the lexer: top-level `;` outside strings, comments, heredocs and tags,
  with the clickhouse-client rule for `INSERT` data (see "Architecture notes").
- Verifier: token streams of input and output equal ignoring whitespace tokens,
  with the case exemption for recorded keyword slots
  (none yet in M2)
  and byte equality for comments and tags;
  re-lexing the output as part of the comparison so a lost newline after a line comment or a glued `--` shows up.
- Safe cleanup only: strip trailing whitespace, collapse runs of blank lines
  (per the blank-line decision), ensure exactly one final newline, normalise CRLF to LF.
  No re-indentation, no keyword case.
- CLI: `chl [PATH…]` formats in place; `--check` lists unformatted files and exits 1;
  `--diff` prints unified diffs; `-` reads stdin and writes stdout;
  `--line-width` is accepted and stored for later milestones.
  File discovery: `*.sql`, `*.sql.j2`, `*.j2.sql`, honouring `.gitignore` via the `ignore` crate.
  Exit codes: 0 clean, 1 `--check` found differences, 2 errors (unreadable, non-UTF-8, lexer error, verifier failure).
  A verifier failure never writes the file and asks for a bug report.
- Library API: `chairlift::format(source: &str, options: &Options) -> Result<String, Error>`
  and `chairlift::check(...)`, with `Options { line_width: u16, jinja: bool }`.
- A corpus tool (`cargo run --bin corpus` or an xtask): runs format, verify,
  idempotence and optionally the oracle over a directory tree, with per-file timing and a summary table
  (duckdb-friendly CSV).

Acceptance:

- A deliberately injected printer bug (print `-` and `-1` adjacent) is caught by the verifier in a test.
- Over every corpus: verify passes, output is idempotent,
  and `clickhouse-format` canonical forms are unchanged for every file the oracle accepts, with the allowlist empty.
- `chl --check` on the stateless corpus runs in under 5 s in release mode.
- The DESIGN.md example is unchanged by M2 (it already has no trailing whitespace or blank-line runs).

Effort: 2–3 days.

### M3. Parser core: SELECT, CREATE TABLE, CREATE MATERIALIZED VIEW

Goal: a tolerant parser with the fallback ladder,
producing a lossless CST for the three statements the corpora are made of, with everything else a verbatim statement.

Scope:

- Event-based parser with checkpoints and rollback; `rowan` tree built from the events.
- Expressions: a Pratt parser with ClickHouse's operator precedence
  (`::` tightest, then unary, `*`, `+`, comparison, `BETWEEN`, `LIKE`/`IN`, `NOT`, `AND`, `OR`, `?:`),
  function calls (including parametric `f(a)(b)` and `-If`/`-State` combinators as plain names),
  lambdas `x -> …` and `(a, b) -> …`, tuples, arrays, maps, `CASE`, `INTERVAL n UNIT`, `{name:Type}`,
  `x.1` tuple access, aliases with and without `AS`, `* EXCEPT/REPLACE/APPLY`, `COLUMNS(...)`, `CAST(x AS T)`,
  window `OVER (...)` kept verbatim.
- `SELECT`/`WITH`: `WITH` (CTEs and `expr AS name`), `DISTINCT`, select list, `FROM`
  (tables, subqueries, table functions, `FINAL`, `SAMPLE`),
  joins (all kinds incl. `ASOF`, `ARRAY JOIN`), `PREWHERE`, `WHERE`, `GROUP BY`
  (with `WITH TOTALS/ROLLUP/CUBE`, `GROUPING SETS`),
  `HAVING`, `WINDOW`, `QUALIFY`, `ORDER BY`
  (with `COLLATE`, `WITH FILL`),
  `LIMIT n BY`, `LIMIT`/`OFFSET`, `SETTINGS`, `FORMAT`, `UNION`/`INTERSECT`/`EXCEPT`, `INTO OUTFILE` verbatim.
- `CREATE [OR REPLACE] [TEMPORARY] TABLE [IF NOT EXISTS] [db.]name [ON CLUSTER c]`: column list with types
  (nested parens),
  `NULL`/`NOT NULL`, `DEFAULT`/`MATERIALIZED`/`ALIAS`/`EPHEMERAL`, `CODEC`, `TTL`, `COMMENT`, `INDEX`, `PROJECTION`,
  `CONSTRAINT`; `ENGINE = name(args)`; `PARTITION BY`, `PRIMARY KEY`, `ORDER BY`, `SAMPLE BY`, `TTL`
  (list of rules), `SETTINGS`, table `COMMENT`; `AS SELECT`, `AS table`, `AS table_function`.
- `CREATE MATERIALIZED VIEW … [TO table | ENGINE … ] [POPULATE] AS SELECT`.
- The keyword classification rule and the positional rules from "Architecture notes";
  the parser records which tokens it consumed in keyword slots so the verifier and the printer can act on them.
- Tree inspection: a `tree` subcommand or test helper that dumps the CST with verbatim regions marked,
  and a statistic of parsed versus verbatim statements, clauses and items per corpus.

Not in scope: layout (M4), any other statement kind (M7), structural Jinja.

Acceptance:

- No panics and verify passes on every corpus file; idempotence is trivially true because M3 prints the tree losslessly
  (the printer comes in M4), so the acceptance is on the tree: `concat(tree) == input`.
- The DESIGN.md example, chum's and the workshop's `CREATE TABLE`,
  materialized views and `SELECT`s parse with no verbatim fallback except where the construct is listed as out of scope.
- On the stateless corpus, the share of `SELECT` and `CREATE TABLE` statements
  that parse without any verbatim fallback is reported and is at least 90% for `SELECT` and 95% for `CREATE TABLE`.
- Keyword-slot classification validated through M4's oracle run; in M3 a unit-test table covers the known traps:
  `ENGINE = Null`, a column named `Settings`, a table named `union`, `SELECT interval FROM t`, `primary Int8`,
  `left(`/`any(`/`format(` as functions, `WHERE (`, `SELECT 1 AS limit`, `GROUP BY table`.

Effort: 6–8 days.

### M4. Formatting rules and printer for M3 statements

Goal: the style decisions above, applied to `SELECT`, `CREATE TABLE` and `CREATE MATERIALIZED VIEW`,
with comments and blank lines where the author put them.

Scope:

- The document IR and printer described under "Architecture notes".
- Layout rules per the "Style" decisions and the layout proposal
  (sections 2 and 3),
  covering: statement and clause lines, lists and the SELECT-list rule, conditions, trailing commas, column definitions,
  blank lines, comments, hugging of subqueries, CTEs and projections, `CASE`, lambdas, joins and `ON`,
  `INSERT … SELECT` shape prepared for M7, the ClickHouse-specific clauses, `;` placement.
- Keyword re-casing at keyword slots; literal casing (`true`/`false`/`inf`/`nan` lower, `NULL` upper).
- Verbatim regions re-indented as a block; `::` left operand and `INSERT` data untouched.
- Snapshot tests (`insta`) per fixture; the DESIGN.md example and the layout proposal's examples as golden files
  (every example in the proposal is valid ClickHouse and oracle-checked).
- Idempotence and oracle runs in the corpus tool and in CI.

Acceptance:

- The DESIGN.md example formats to the proposal's section 4 output byte for byte.
- Every example in the layout proposal that uses M3 statements formats to the recommended alternative.
- Over every corpus: verify passes, output is idempotent, every comment and tag is emitted once.
- Oracle: canonical forms unchanged for every accepted file in the local corpora,
  and at most 50 differing files on the stateless corpus, each classified
  (reserved-word-as-identifier cases to fix in M7, or an allowlisted oracle quirk).
  The spike reached 43 with cruder rules.
- `chl --check` over the stateless corpus in under 15 s single-threaded, release build; per-file p95 under 5 ms.

Effort: 5–7 days.

### M5. Jinja templates

Goal: chum's and dbt's templates format the way the decisions describe,
and the tests prove the rendered SQL is unchanged.

Scope:

- Parser: block tags and Jinja comments as trivia with the clause fallback;
  `{{ }}` as an atom in expression and name positions, including in table names, `ENGINE` arguments,
  `GRANT … TO {{ role }}`, and inside `SETTINGS` values.
- Layout: block tags on their own line at the enclosing indent with closers aligned to their opener;
  tags between statements at column 0; a `{% for %}` emitting list items at the list's indent;
  `{% set %}` and `{{ config() }}` headers at column 0 with their blank lines kept.
- Whitespace-control safety: the text next to `{%-`/`-%}`/`{{-`/`-}}` is never moved across a line break by the
  formatter (the render would then join tokens, which the token check cannot see); a unit test reproduces sqlfmt's
  failure where a `--` comment line followed by `{%- if %}` swallowed the next line.
- Render harness in tests: minijinja with chum's environment settings and stub filters,
  rendering every template for each combination of its `{% if %}` branches
  (cap at 2^4, sample beyond), then the oracle on input and output.
- Jinja mode by extension in the CLI; the library takes it as an option.

Acceptance:

- chum's template fixtures, the private `.up.j2.sql`
  and the dbt samples format without verbatim fallback at the statement level,
  except the documented `{% if %}where{% else %}and{% endif %}` shape, which stays verbatim.
- For every rendered variant that the oracle accepts, canonical forms of input and output are equal.
- The DESIGN.md example with its Jinja lines formats to the proposal's output.
- A template with a tag inside a string literal, a tag glued to an identifier, `{# #}` comments,
  `{%+ %}` and `{% raw %}` round-trips through verify unchanged in all tag bytes.

Effort: 3–4 days.

### M6. CLI polish and performance

Goal: a binary comfortable in an editor's format-on-save and in pre-commit over a whole repository.

Scope:

- `--verify` (removed after M8; see "Verification"):
  the oracle check through a `clickhouse-format` found on `PATH` or given with `--clickhouse-binary`,
  batched per process with per-file fallback, with the error classification from "Corpora and tooling";
  templates are rendered first if minijinja is compiled in (`template` feature) or skipped with a note.
- Parallel file processing (`rayon`) and a quiet/verbose output with per-file diagnostics via `miette`, as chum does.
- `--stdin-filename` so an editor can pass the extension for Jinja mode; `--line-width` applied.
- Document-building performance work from the spike's list: cached per-node facts, no per-token `String`.
- Benchmarks in CI with a regression threshold.

Acceptance:

- Formatting a 1 MB file in under 100 ms and `--check` over the stateless corpus in under 5 s on 8 cores.
- `--verify` (removed after M8) over the local corpora reports zero differences
  and classifies the known unparseable files correctly.
- `echo 'select 1' | chl -` prints `SELECT 1` and exits 0; a lexer error exits 2 with a position.

Effort: 2 days.

### M7. Remaining statement kinds

Goal: every statement that appears in the corpora is parsed and laid out, in the order of its frequency, with `SYSTEM`,
`EXPLAIN` and anything rarer staying verbatim.

Scope, in this order (each item: parse, layout, fixtures, oracle run):

1. `INSERT INTO [db.]t [(cols)] { VALUES … | FORMAT f … | SELECT … }` with the data verbatim
   and the `INSERT … SELECT` shape.
2. `DROP`, `SET`, `TRUNCATE`, `DETACH`/`ATTACH`, `RENAME`, `EXCHANGE`, `OPTIMIZE`, `USE`:
   one-line statements with a parsed head and a verbatim tail; they mostly need keyword re-casing and a fit rule.
3. `ALTER TABLE` with its command list
   (`ADD/DROP/MODIFY/RENAME COLUMN`, `MODIFY TTL`, `MODIFY SETTING`, `ADD INDEX/PROJECTION`,
   `UPDATE`/`DELETE` mutations, partition commands verbatim).
4. `CREATE VIEW`, `CREATE DATABASE`, `CREATE DICTIONARY`
   (source, layout, lifetime clauses as parsed one-liners), `CREATE FUNCTION`.
5. `GRANT`/`REVOKE`, `CREATE/ALTER/DROP ROLE|USER|QUOTA|ROW POLICY|SETTINGS PROFILE`.
6. A pass over the stateless oracle differences left from M4: fix every reserved-word-as-identifier case.

Acceptance:

- Every statement in the local corpora parses without statement-level fallback.
- On the stateless corpus, at most 5% of statements fall back at the statement level,
  and the oracle differences are down to the allowlisted quirks.
- Each statement kind has at least one fixture in the snapshot suite and one in the layout proposal's style.

Effort: 4–6 days.

### M8. Hardening and first release

Goal: `chairlift 0.1.0` as a static binary and a crates.io library, safe enough to run on migrations people depend on.

Scope:

- Fuzz targets for lexer, parser and the whole `format`
  (no panics, verify holds, idempotent), run for an hour before release and in a weekly CI job.
- The docs-examples corpus added to the oracle sweep, filtered by oracle acceptance.
- README with installation (prebuilt binaries through mise and cargo-binstall, as in chum; no installer scripts), usage,
  the style summary, editor setup (stdin mode), and the pre-commit snippet; `CHANGELOG.md` managed by release-plz.
- A `--version` that prints the ClickHouse keyword-list version it was built against.

Acceptance:

- A tagged release with binaries for Linux (glibc and musl), macOS and Windows, published by the cargo-dist workflow.
- All corpus checks green; the oracle sweep green with the documented allowlist.

Effort: 2 days.

### M9. chum integration

Goal: chum can format and check its migrations with the library.

Scope:

- chum depends on the published `chairlift` crate; `chum fmt [--check]` over a migration directory,
  and an optional formatting check when migrations are loaded (`chum check`).
- chum's statement splitter can switch from sqlparser's tokenizer to chairlift's lexer,
  which also removes the sqlparser dependency; both splitters are run side by side over chum's fixtures first.
- Decide about moving into a workspace; nothing in this plan requires it.

Risk, out of scope here: chum's checksum is over the source text,
so formatting an applied migration changes its checksum and chum refuses to run.
Adopting `chum fmt` on an existing project needs either a formatting-insensitive checksum in chum
(for example over chairlift's token stream with whitespace removed, which the lexer makes cheap)
or a one-off re-checksum command.
That is a chum decision to take before M9 ships.

Acceptance:

- `chum fmt --check` passes on chum's own fixtures after one `chum fmt`.
- chum's test suite passes with the chairlift-based splitter.

Effort: 2 days, plus the chum-side checksum work if it is taken.

### Later, not planned

- Structural Jinja (`if`/`for` as tree nodes) if real templates split expressions or words across branches.
- A tree-sitter grammar for editor highlighting, built from chairlift's fixtures.
- Lint rules on the CST (for example `ORDER BY tuple()` warnings), a language server.
- Byte-level (non-UTF-8) input.

## Open questions

Decisions still needed from the project owner, by milestone:

- **M2**: Should CRLF files be written back as CRLF, or always LF?
  (Plan assumes LF; chum normalises CRLF for its checksum.)
- **M2**: Exit code and message when a file is left unchanged because of a lexer error: error (2) or a warning with 0?
  (Plan assumes 2.)
- **M4**: `GROUP BY hour, site WITH TOTALS`: on the `GROUP BY` line when it fits, own line otherwise
  (the proposal's rule), or always on its own line?
- **M5**: The cap on rendered branch combinations in the test harness (plan: 16 per template).
- **M9**: Whether chum gains a formatting-insensitive checksum (see the risk above).

## Spike reports

All under `docs/spikes/`, dated 2026-11-10.
Scratch paths they mention were in a job directory that no longer exists.

| Report | What it settled |
| --- | --- |
| `2026-11-10-handwritten-pipeline.md` | The architecture works end to end on 12,610 corpus files; crate choices; fallback ladder; comment handling; the keyword-slot problem; timings; effort estimates |
| `2026-11-10-tree-sitter-topiary.md` | Why Topiary is not the formatting engine; what a tree-sitter grammar would cost and give |
| `2026-11-10-clickhouse-internals-and-corpus.md` | ClickHouse's lexer rules and token list, keyword handling, what the oracle re-cases and hides, corpus sizes, statement frequencies, oracle timing and edge cases |
| `2026-11-10-prior-art.md` | sqlfmt's and sqlfluff's Jinja handling and failure modes, sqlglot's and AfterShip's grammar coverage, lexer traps, how Black/ruff/Prettier/StyLua verify, Rust crates, dbt corpus |
| `2026-11-10-prior-art-addendum.md` | minijinja as a tag finder (rejected), chum's render-then-split pipeline and checksum, release-plz per-package tags |
| `2026-11-10-layout-proposal.md` | The style sheet with rendered alternatives for every open layout question, and the DESIGN.md example as it should print |
