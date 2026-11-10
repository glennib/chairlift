# Spike: hand-written pipeline (lexer → rowan CST → Doc IR → printer → verifier)

Spike crate: `/home/glenn/.claude/jobs/ab5633ea/tmp/spike-handwritten/spike`
(run `CARGO_TARGET_DIR=../target cargo build --release`).
Binary: `target/release/chairlift_spike {fmt|check|bench|tree|pretty} FILE…`; `biome` subcommand with `--features biome`
(built into `../target-biome`).
Oracle scripts: `oracle.sh` and `oracle1.sh` (plain SQL), `oracle_j2.sh`
(templates, rendered with minijinja in two contexts).

## Recommendation

The architecture holds, and it is the right route.
In about 2,500 lines, the spike covers every item in the brief:

- It is lossless and passes the token verifier and idempotence on all 12,610 UTF-8 files of ClickHouse's `0_stateless`
  (no panics).
- The `clickhouse-format` oracle passes on all 17 of our plain-SQL corpus files it accepts,
  and on 12,005 of the 12,048 stateless files it accepts.
- It formats the DESIGN.md example as intended.

Choices:

- **Lexer:** hand-written.
- **CST:** `rowan` 0.16.1, with an event list in front so the parser can backtrack.
- **Printer:** our own (about 210 lines).
  Keep the IR shaped so `pretty` could replace the printer, but don't depend on biome.

Build the verifier and the oracle harness first.
They found every real bug in the spike, including two the design didn't predict: printer cleanup turning `#` into `#`,
and Unicode whitespace being dropped.

The hard part is not the machinery.
It is two things the token check can't see:

1. **Keyword vs identifier by position.**
   Examples: `ENGINE = Null`, a column called `Settings`, a table called `union`.
   Only the oracle catches these.
2. **Places where ClickHouse whitespace does matter.**
   Examples: the operand text of `literal::Type`,
   and inline data after `INSERT … VALUES` running to the end of the line.

## Findings

### Correctness over corpora (release build, width 100)

| Corpus | Files | Lossless lex | Verify + idempotent | Oracle same | Oracle rejects input | Oracle diff |
| --- | --- | --- | --- | --- | --- | --- |
| chum + tally + workshop plain SQL, plus `samples/comments.sql` | 19 | 19 | 19 | 18 | 1 (INSERT VALUES with a comment inside) | 0 |
| Templates (chum, tally, DESIGN, 2 own case files), each rendered "full" and "min" | 8 | 8 | 8 | 12 of 16 renders | 4 renders (my contexts lack vars, or the branch combination is invalid SQL) | 0 |
| ClickHouse `0_stateless` | 12,619 (9 not UTF-8) | 12,610 | 12,610 | 12,005 | 571 | 43 |

Before the fixes, the stateless corpus gave 8 verify failures, 5 idempotence failures and 133 oracle diffs.
The causes and fixes are in "Nastiest problems".

The 43 remaining oracle diffs:

| Cause | Files |
| --- | --- |
| Reserved word used as an identifier (`select`, `join`, `from`, `union`, `null`, `interval`, `All`) and re-cased | ~13 |
| `[3,4,5]::Array(Int64)`: ClickHouse turns this into `CAST('[3,4,5]', …)` using the literal's source text, so `[3, 4, 5]` canonicalizes differently | 11 |
| `INSERT … VALUES (1); select 1;` on one line: clickhouse-format treats the rest of the line as data, so moving `select` changes the split | 12 |
| Non-UTF-8 files (fmt skipped them, so the comparison is empty) | 7 |

### Speed (release, idle machine)

| Input | Lex | Parse | CST→Doc | Print | Verify (re-lex output + compare) |
| --- | --- | --- | --- | --- | --- |
| Largest corpus file, `health.sql` (6.9 KB, 1,194 tokens) | 16 µs | 129 µs | 315 µs | 35 µs | 77 µs |
| Synthetic 1.0 MB (our corpora ×N, 134k tokens) | 1.1 ms (950 MB/s) | 6.3 ms (163 MB/s) | 28 ms | 5.2 ms | 8.3 ms (123 MB/s) |
| 4.7 MB of `0_stateless` (1.55M tokens, 3 bytes/token) | 15 ms (312 MB/s) | 93 ms (50 MB/s) | 505 ms | 91 ms | 132 ms (35 MB/s) |

- **End to end:** `fmt` on the 1 MB file takes 0.11 s with 27 MB RSS.
  `check` (format twice plus verify) over all 12,619 stateless files takes 13.7 s.
- **The verifier needs no off switch:** about 80 ns per token.
- **CST→Doc is the slowest stage.**
  It rescans subtrees at every level and allocates a `String` per token; caching per node fixes that.
- **The parser compares keywords case-insensitively at every peek.**
  That is why it slows on dense input.

### Printer candidates

| | `biome_formatter` 0.5.7 | `pretty` 0.12.5 | Own printer |
| --- | --- | --- | --- |
| Published | Yes, but last released 2024-03-12. The biome repo no longer publishes these crates. | Yes, maintained | n/a |
| Builds as-is | No. Semver-compatible 0.5.8 releases of `biome_rowan`/`biome_diagnostics`/… break it, so 8 crates must be pinned with `=0.5.7` / `cargo update --precise`. | Yes | Yes |
| Dependencies (normal, unique) | 105 (whole crate: 107 vs 11) | 4 | 0 |
| Clean release build of the spike | 30.5 s | 8.1 s (with rowan + minijinja) | same |
| Standalone use | Works via `SimpleFormatContext` and `format!`. Has `line_suffix`, `expand_parent`, `empty_line`, best-fitting. | Plain combinators. No line-suffix and no break propagation; I emulated them with `flat_alt(fail())`. Writes indentation on blank lines. | All of it, and `{% end… %}` aligned to its opener |
| Same output as own printer on corpus + cases | All files except the 2 `{% endif %}` lines (no opener alignment). Needed `empty_line()` for blank lines. | All files except the same 2 lines | reference |

`pretty` is an acceptable fallback.
I don't recommend biome: it is a stale, pinned, 100-crate dependency for features we already have in 200 lines.

### Fallback granularity (what I ended up with)

The parser emits events (`Start`, `Tok`, `Finish`) into a vector.
A checkpoint is (event count, token position), and rollback truncates.
The rowan tree is built from the events at the end; rowan's builder can't roll back, so the events stay outside it.

The fallback is a three-step ladder, all ending in VERBATIM nodes:

1. **List item** (column definition, select item, function argument, tuple or array element, setting, FROM table).
   The item is parsed.
   If it doesn't end at `,`, the list end, `;` or EOF,
   it is rolled back and becomes VERBATIM up to the next `,` or closing bracket at paren depth 0.
   Example: `extract(day from ts)` keeps `extract(…)` structured; only the argument `day from ts` is verbatim.
2. **Clause** (SELECT, FROM, WHERE, GROUP BY, ORDER BY, LIMIT, SETTINGS, the JOIN forms, ENGINE, PARTITION BY, TTL, …).
   If the body parses but isn't followed by a clause keyword or the statement end,
   the whole clause rolls back and becomes VERBATIM up to the next clause keyword at depth 0.
   Clauses may come in any order and repeat.
   That is why the `{% if %} WHERE … {% else %} WHERE … {% endif %}` case parses.
3. **Statement.**
   Anything that isn't CREATE [OR REPLACE] [TEMPORARY] TABLE,
   CREATE MATERIALIZED VIEW or SELECT/WITH is VERBATIM up to the next `;` token.
   Strings, comments and Jinja are already single tokens, so the split is safe.

A failing expression costs at most one list item.
Verbatim keeps its original keyword case.
On stateless, 39% of top-level statements (27% of bytes) are verbatim: SET, DROP, INSERT, SYSTEM, ….

### Comments

There is no attachment step.
Trivia (comments and `{% %}`/`{# #}` tags) stay in the CST where the parser flushed them: before starting a node,
so between siblings of the parent.

The formatter classifies each one by the newlines before it:

- **No newline (trailing):** stays on its line, after the comma.
- **Newline (own-line):** goes on its own line before the next item.
- **Two or more newlines:** one blank line is kept.

Positions it handles: between statements, clauses, column-list or select-list items, and AND/OR operands.
Anywhere else (inside a call, between `ORDER` and `BY`, after `AND`),
the enclosing item or clause body is printed verbatim.

What broke:

1. A trailing comment after a list item made the item's call arguments break.
   Fixed in `fits()`.
2. A comment after `AND` would have moved before it.
   The chain now refuses, so the expression goes verbatim.
3. `SELECT -- c` moves the comment to the next line.
   Cosmetic and still open.
4. Trimming turned `#` into `#`; see "Nastiest problems".

### Jinja

- **Lexing:** `{{ }}`, `{% %}` and `{# #}` are single tokens.
- **Glued tags:** a tag glued to word characters becomes part of the word
  (`events_{{ s }}`, and also `events{% if v2 %}_v2{% endif %}`, so formatting can never split a rendered identifier).
- **Inside strings:** a tag inside a string stays in the string, and quotes inside the tag don't end the string.
- **How the parser sees it:** `{% %}` is trivia, so the parser sees the SQL of all branches together.
- **Layout:** block tags are always printed on their own line.
  `{% end… %}`, `{% else %}` and `{% elif %}` take their opener's indentation.

Results:

- The DESIGN.md example formats as intended (example A).
- **if/else between two whole WHERE clauses:** formats cleanly (example B).
- **`{% if %}where{% else %}and{% endif %}`:** the glued word makes that clause verbatim.

For every template whose renders `clickhouse-format` accepts, both renders are unchanged by formatting.

## Output examples

A. The DESIGN.md input, after formatting (the header comment and GRANT are unchanged):

```sql
CREATE TABLE analytics.page_views_1h
(
    hour DateTime('UTC') CODEC(Delta, ZSTD(1)),
    site LowCardinality(String),
    views UInt64,

    -- A comment inside a column list must survive.
    visitors AggregateFunction(uniq, String)
)
ENGINE = AggregatingMergeTree
PARTITION BY toStartOfDay(hour)
ORDER BY (site, hour)
TTL hour + INTERVAL 90 DAY
SETTINGS ttl_only_drop_parts = 1;

CREATE MATERIALIZED VIEW analytics.page_views_1h_mv TO analytics.page_views_1h AS
SELECT
    toStartOfHour(event_time) AS hour,
    site,
    count() AS views,
    uniqState(visitor_id) AS visitors
FROM raw.events
WHERE true
    AND event_type = 'page_view'
    {% if this.cutoff is defined -%}
    AND event_time >= {{ this.cutoff | sql_datetime }}
    {% endif -%}
GROUP BY hour, site;
```

B. `{% if %}`/`{% else %}` choosing between whole clauses:

```sql
FROM t
{% if full %}
WHERE a = 1
{% else %}
WHERE a = 2 AND b = 3
{% endif %}
ORDER BY a;
```

C. Breaking at width 100, and a long SETTINGS list:

```sql
SELECT
    someVeryLongFunctionName(
        argument_number_one,
        argument_number_two,
        argument_number_three
    ) AS result
FROM t
WHERE (first_condition = 1 OR second_condition = 2)
    AND third_condition_is_long = 'value'
    AND fourth = 4
SETTINGS
    index_granularity = 8192,
    storage_policy = 'hot_cold_with_a_long_policy_name_that_does_not_fit',
    min_bytes_for_wide_part = 0;
```

D. Fallback at each level.
The INDEX item, the argument `day from ts`, the window spec and the unknown `INTO OUTFILE` clause stay as written:

```sql
    index idx a type minmax granularity 1,
    extract(day from ts) AS d,
    sum(v) OVER (partition by k order by ts),
into outfile 'x.csv';
```

## Lines of code and crates

- **Lines of code:**
  - lexer 420 (incl. tests);
  - parser 1,050 (incl. ~70 lines of keyword tables);
  - Doc IR + printer 220;
  - CST→Doc rules 580;
  - verifier 30;
  - adapters 37 (`pretty`) / 52 (biome);
  - CLI + renderer 133.
- **`rowan` 0.16.1:** used.
  It didn't fight: one `Language` impl.
  Backtracking and trivia placement live in an event layer in front of it.
- **`minijinja` 3.1.0:** test renderer only.
- **`logos`:** not used.
  The hard parts need context or callbacks: Jinja tags with their own strings, tags glued into words,
  `$tag$` back-references, nested `/* */`, `.` vs `.5`, `#` only before a space or `!`, Unicode whitespace.
  The hand-written lexer runs at 300–950 MB/s.

## Nastiest problems

1. **Keyword vs identifier.**
   ClickHouse decides by position: `Null` the engine, `Settings[…]` the column, `union` the table,
   `left(`/`any(`/`format(` the functions, `WHERE (` the clause.
   - The token verifier trusts the parser, so it is blind here.
     The oracle found 133 files.
   - Two positional rules took that to 43: an ENGINE takes a name, not an expression;
     SETTINGS is a clause only before `name =`.
   - The real parser needs positional keyword rules per clause, and the oracle in CI.
2. **Whitespace that matters to ClickHouse:**
   - The operand of `::` keeps its source text, so `[3,4,5]::Array(Int64)` reaches ClickHouse as the string `'[3,4,5]'`.
   - Inline INSERT data runs to the end of the line.
   - Rules for the real implementation: keep the left operand of `::` verbatim,
     and keep everything from VALUES/FORMAT to the end of that line verbatim.
3. **Printer and gluing edge cases that silently change tokens.**
   All three were caught by the verifier.
   - `- -1` was printed as `--1`.
   - Trimming `#` turned a comment into an error token.
   - Unicode whitespace was dropped by `trim()`.
   - The general fix: the printer may glue two tokens only if their concatenation re-lexes to the same two tokens,
     and layout spaces are emitted lazily so token text is never trimmed.
     The injected `-`/`-1` bug was caught: `token 2: input OP "-" vs output LINE_COMMENT "--1,"`.

## What I'd do differently

- Classify keywords into an enum in the lexer.
- Cache per node: "contains trivia", first token, last token.
- Run the stateless oracle in CI from day one (about 6 minutes on 16 cores).
- Attach comments per list, so a comment inside call arguments doesn't force verbatim.

## Estimated effort for the real implementation (focused agent-days)

| Stage | Days |
| --- | --- |
| Lexer: full ClickHouse parity (Unicode quotes `‘’“”`, `\G`, …) plus fuzzing | 1–2 |
| Parser: CREATE TABLE, MV, SELECT at stateless level, plus positional keyword rules | 8–12 |
| Formatting rules, comments, Jinja, snapshot tests, taste iteration | 6–10 |
| Printer | 1 |
| Verifier, oracle harness, render check, corpora in CI | 2–3 |
| CLI, file discovery, `--check` / `--diff` | 1–2 |
| **Total** | **19–30** |

## Verified / Unverified

- **Verified (run):**
  - every number in the tables above;
  - the oracle runs, plain and rendered;
  - the injected `--` bug being caught;
  - the `pretty` and biome adapters reproducing our output;
  - biome 0.5.7 needing pins to build;
  - `clickhouse-format`'s INSERT line splitting;
  - the ClickHouse lexer rules, read in the local clone's `src/Parsers/Lexer.cpp`.
- **Unverified:**
  - logos speed;
  - whether `literal::T` spacing is harmless on a server (likely, since ClickHouse re-parses the string);
  - server behaviour for INSERT data on the same line;
  - `{%- -%}` render cases (not in the corpus);
  - the effort estimates.

## Corpus notes (from reading the files)

| Observation | Where | What it needs, and what the spike did |
| --- | --- | --- |
| `{% if %}…{% endif %}` wraps a whole statement at top level | chum `0001_base.up.sql.j2` | Tags between statements. Handled: tags are trivia at root level, aligned to column 0. |
| `where true and …` with an `{% if -%}` guarding one `and` term; the file ends without `;` | tally `0003_product_features.up.j2.sql` | Same shape as DESIGN.md. Handled, and the render oracle passes both ways. |
| `BETWEEN {from:Date} AND {to:Date}`, `{name:Type}` parameters | workshop `dashboard/query.sql` | The BETWEEN `AND` must not become a chain break (correct), and parameters are atoms. Handled. |
| `table`, `type`, `value`, `count`, `rows` as column names (`GROUP BY table`) | workshop `health.sql` | Re-casing depends on position. These came through unchanged (oracle: same). |
| A trailing `--` comment after a select item's comma | workshop `health.sql` | Kept after the comma (this is what exposed the `fits()` bug). |
| Hand-aligned column types; comment lines above columns | workshop `schema.sql`, chum `initial_schema` | Comments kept. Alignment collapsed (open question 2). |
| A long `SETTINGS` already written one per line | tally `0001_landing_events.up.sql` | Unchanged by the fit-or-one-per-line rule. |
| `DEFAULT -1`, `DEFAULT x CODEC(…)`, `DEFAULT 'a;b;c'` | tally 0001, chum `initial_schema` | Unary minus, CODEC after DEFAULT, and `;` inside a string. All handled. |
| `CREATE DICTIONARY … LIFETIME(MIN 0 MAX 0)`, `ALTER TABLE … ADD COLUMN … AFTER` | chum, workshop | Verbatim statements, case kept. |

## Open questions for the orchestrator

1. **SELECT layout.**
   I copied clickhouse-format: one item per line whenever there is more than one.
   GROUP BY, ORDER BY and SETTINGS fit on one line or go one per line.
   Is that the style we want?
2. **Alignment.**
   Hand-aligned column types (`event_time    DateTime64`) are collapsed to one space.
   Keep alignment, or collapse it?
3. **Statement endings.**
   `AS` goes at the end of the CREATE … AS line, with SELECT starting at column 0.
   `;` goes at the end of the last line, or on its own line after a trailing comment.
   OK?
4. **Two whitespace-sensitive cases.**
   Should verbatim around the left operand of `::` and INSERT inline data be required rules from milestone 1?
5. **Jinja reach.**
   Is tag-as-trivia, with opener-aligned closers, enough Jinja support for v1?
