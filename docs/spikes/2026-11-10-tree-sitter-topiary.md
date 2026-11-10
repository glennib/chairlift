# Spike: tree-sitter + Topiary for chairlift

## Recommendation

Build the hand-written Rust parser with chairlift's own Wadler-style printer.
Don't use Topiary as the formatting engine.

Tree-sitter did better than DESIGN.md feared.
The spike grammar needed 0–3 declared conflicts,
and a GLR "opaque statement" fallback kept every corpus file free of ERROR nodes.
Topiary is what rules this route out:

- **No width-aware layout.**
  Every line break depends on the input: one newline anywhere in a node makes the whole node multi-line,
  and a 212-character one-line SELECT stays on one line.
  Line-width wrapping has been an open issue (topiary#700) since 2024.
- **The default drops all whitespace.**
  Any token pair that no query covers gets glued together.
  With an empty query file, `-- c1\nselect a, b …` came out as `-- c1selecta,bfrom…`:
  the code was swallowed into a comment, and the result still passes Topiary's idempotence check.
- **ERROR handling is all-or-nothing, and unsafe when tolerated.**
  Without `-t`, any ERROR node rejects the whole file (exit 5).
  With `-t`, ERROR nodes are kept verbatim but glued to their neighbours, which changes meaning: `GRANTSELECT`,
  `TOreader`, `ALTERTABLE`, `WHEREa`.
  One input failed Topiary's idempotence check.
- **Injections can't handle Jinja interleaved with SQL.**
  Each injected span is formatted independently, and there is no `injection.combined`.

If editor highlighting matters later,
a tree-sitter grammar can be a separate side project that doesn't block the formatter.
A hybrid (tree-sitter CST plus our own printer) would work but keeps the lexer limits listed below and a C build.
It buys little over a hand-written parser, apart from GLR fallback and highlighting.

### Main risk of each route

- **Hand-written parser:** the effort and coverage of a large, growing grammar.
  This is mitigated by verbatim fallback at whatever granularity we choose.
- **tree-sitter + Topiary:** the layout-quality ceiling
  (no width-aware layout),
  and safety that depends on every query being complete. chairlift's token verifier would be mandatory
  and would fire often.

## 1. Existing grammars

| Grammar | Rules | Keyword rules | Conflicts | `word` | parser.c | Activity | ClickHouse coverage |
|---|---|---|---|---|---|---|---|
| r-k-jonynas/tree-sitter-clickhouse | 48 | inline `case_insensitive()` regex | 1 | none | 548 KB | 2 stars, last commit 2025-11 | CREATE TABLE (ENGINE must directly follow the columns), SELECT (FROM required), INSERT, WITH. No MV, TTL or CODEC. Errors in 19/20 corpus files |
| DerekStride/tree-sitter-sql (crate `tree-sitter-sequel` 0.3.12) | 680 | 382 `keyword_*` via `make_keyword()` regex | 10 | `_identifier` | **44.7 MB**, 32,032 states | active (2026-11) | Generic SQL with no ClickHouse mentions. Fails on `DateTime('UTC')`, `CODEC(…)`, `TTL … SETTINGS`. Errors in 17/20 files |
| m-novikov/tree-sitter-sql | 190 | 264 `kw()` aliases of regex | 0 | none | 35.8 MB | Postgres-focused; last commit 2024-03; `.rej` files in `src/` | none |

There is no `tree-sitter-clickhouse` on npm or crates.io.

**How keywords work:** case-insensitive keywords are written as character-class regexes (`[sS][eE]…`).
With `word:` set, tree-sitter uses keyword extraction:
a keyword is only lexed where it is valid in the current parse state, and otherwise the word is lexed as an identifier.
That makes `key`, `value`, `date` and `order` usable as column names for free (tested).

The cost of this technique:

1. A keyword that is valid at the same position as an identifier always wins.
   `primary Int8` and `SELECT interval FROM t` fail, although ClickHouse accepts both (verified).
   Each such keyword needs an explicit `alias(kw('X'), $.identifier)`.
2. Regex keywords tie with the identifier regex on length, and the tie goes to rule order.
   In the spike, `TRUE` and `FALSE` were **silently never lexed**, with no warning,
   until every keyword got `token(prec(1, …))`.

## 4. Hands-on spike grammar

The grammar covers CREATE TABLE (typed columns, CODEC, ENGINE/PARTITION BY/ORDER BY/TTL/SETTINGS),
CREATE MATERIALIZED VIEW, and SELECT with aliases, AND/OR and GROUP BY.
Jinja is handled as follows: `{{ }}` is an atom where a name can stand, and `{% %}` / `{# #}` are extras,
the same mechanism as comments.

| | A: strict | B: + `opaque_statement` fallback |
|---|---|---|
| Rules / grammar.js lines | 51 / 131 | 53 / 134 |
| Declared conflicts | **0** (after one grammar fix for a `{{ }}` ambiguity) | **3** (14 until statements required `;` separators) |
| `tree-sitter generate` | 0.30 s | 0.33 s |
| parser.c / states | 403 KB / 355 | 441 KB / 375 |
| DESIGN example, Jinja removed | ERROR on GRANT; `SELECT ON … TO reader` recovered as a bogus `select_statement` | 0 errors; GRANT becomes `opaque_statement` |
| DESIGN example with Jinja | `{% if %}` attached inside the WHERE `binary_expression`, `{% endif %}` at `select_statement` level, `{{ }}` as `qualified_name` | Same, and the GRANT with `{{ }}` becomes `opaque_statement` |
| 20 corpus files + samples, files with ERROR | 17/20 | **0/20** (statements: 29 known, 41 opaque) |
| `{% if %} WHERE … {% else %} WHERE … {% endif %}` | ERROR swallowing 4 statements | whole statement opaque |

How variant B works: `prec.dynamic(-10, repeat1(any_token))` with GLR conflicts.
The fallback works per statement, so one unknown construct
(for example a subquery in `IN`) makes the whole statement verbatim.
Lexical gaps still produce ERROR nodes: `a::String` did.

## 2. Topiary 0.8.0 (binary and library, tested)

| Question | Finding |
|---|---|
| Layout primitives | hardline; empty/spaced softline (breaks if the *parent node spans several lines in the input*); input softline; indent start/end; scoped and measuring scopes; `@leaf`; `@delete`; `@upper_case`; `@allow_blank_line_before`; antispace |
| Width-aware breaking | **None.** Open issue topiary#700 |
| Comments | Ordinary extras nodes handled by your queries. PR #500, which processes comments separately, has been open since 2023. Trailing comments needed special-case queries. An antispace rule for `Type(` also glued `UInt8-- note` until it was anchored |
| ERROR / MISSING | Default: refuses the file on ERROR. `-t`: ERROR becomes a verbatim leaf, but the whitespace around it is lost (`(ERROR) @prepend_space` matched in `-vvvv` logs and had no effect). MISSING nodes are zero-width and skipped, so nothing is inserted |
| Verbatim subtree | Yes, with `@leaf` (used for `opaque_statement`, strings, comments and Jinja) |
| DESIGN example result | Matched the DESIGN layout (output below), was idempotent, and its clickhouse-format oracle output was unchanged. One defect: `{% endif %}` dedented to column 0 because of where the extra was attached |
| Jinja glued to identifier | `raw.events_{{ suffix }}` parsed without error as table `raw.events_` with alias `{{ suffix }}`, and Topiary wrote `events_ {{ suffix }}`, **a silent meaning change** |
| Corpus | 17/17 files formatted. The 12 that ClickHouse can parse are oracle-equal. The 5 Jinja files were checked only for "no failure" |

Output for the plain DESIGN example (abridged):

```text
CREATE TABLE analytics.page_views_1h
(
    hour DateTime('UTC') CODEC(Delta, ZSTD(1)),
    …
)
ENGINE = AggregatingMergeTree
PARTITION BY toStartOfDay(hour)
…
WHERE TRUE
    AND event_type = 'page_view'
    {% if this.cutoff is defined -%}
    AND event_time >= {{ this.cutoff | sql_datetime }}
{% endif -%}
GROUP BY hour, site;
```

## 3. Jinja

There are three options.
The first is the only one that works with Topiary.

- **Inside the SQL grammar (tested):** tags as extras plus a `{{ }}` atom.
  Topiary formats around them.
  The parser sees all branches at once, and clause-choosing `{% if %}` falls to opaque.
  Tags glued to identifiers need lexer work, by making the identifier token absorb `{{…}}`.
- **Jinja as the outer grammar with SQL injected:** this is the precedent. tree-sitter-htmldjango
  and cathaysia/tree-sitter-jinja produce a `content` node with `injection.combined`, which works for editors.
  Topiary has no combined injections and formats each span alone, so `and x >=` becomes a fragment that doesn't parse.
- **dbt-labs/tree-sitter-jinja2:** 172 lines, and its README says "incomplete".
  It parses Jinja only.

## 5. Packaging and speed

| Item | Measured |
|---|---|
| Rust bin: topiary-core 0.8.0 + tree-sitter 0.26.12 + grammar-B parser.c via `cc` | cold release build 24.9 s; rebuild after a grammar change 6.3 s; 3.8 MB binary; only libc and libgcc dynamic |
| topiary-core deps | tokio, rayon, miette, serde_json, prettydiff (56 crates). `default-features = false` fails to compile in 0.8.0 (fixed on main by #1355). Published API differs from main (`formatting_query` is not an `Option`) |
| Compiling DerekStride parser.c (-O2) | **19.3 s, 378 MB RSS, 12.5 MB .so**. A full ClickHouse grammar would land near this size |
| Parse ~100 KB (grammar A / B / DerekStride) | 9–16 / 10–21 / 37–39 ms (5–11 MB/s); noisy machine |
| Topiary format ~100 KB (no idempotence) | 54–120 ms; whole CLI run 0.14 s, 27 MB RSS |

## 6. Verdict

| Criterion | tree-sitter + Topiary | Hand-written Rust |
|---|---|---|
| Tolerance / verbatim fallback | Statement-level GLR fallback works (0 errors on corpus). Finer granularity costs conflicts. Lexical gaps still produce ERROR, which Topiary refuses or corrupts | Any granularity: token-exact verbatim nodes by construction |
| Width-aware layout | None | Wadler printer, our choice |
| Comment preservation | Manual queries per context; glue hazards | Trivia attached by the lexer; verifier-backed |
| Jinja | Extras plus atom works; glued identifiers break; no combined injections | Outer Jinja lexer as DESIGN.md proposes |
| Keyword/identifier | Free via keyword extraction, except keywords valid where an identifier is (explicit aliases); silent regex shadowing | Two-token lookahead; ClickHouse's own approach |
| Build/packaging | C via `cc`; parser.c grows to tens of MB; 20 s+ compile | Pure Rust |
| Editor highlighting | Yes | No (separate grammar later) |
| Maintenance as ClickHouse grows | Conflict churn; states and compile time grow; two DSLs (grammar.js and queries) | Rust code; grows linearly with constructs |

## Verified / Unverified

**Verified (run here):** every number in the tables;
the keyword-as-identifier cases against ClickHouse and the four grammars; Topiary's empty-query gluing;
ERROR refusal and `-t` gluing; MISSING skipping; the `events_{{ }}` meaning change; TRUE/FALSE shadowing;
the oracle check on 12 files; the build and its dynamic deps.

**Read, not run:** Topiary docs and source
(the ERROR-as-leaf code path, `check_for_error_nodes` ignoring MISSING);
PR #500 and issue #700; htmldjango and Jinja injection queries; m-novikov's keyword technique (not compiled).

**Unverified:** a static musl build (no musl target installed); why `(ERROR) @prepend_space` had no effect;
hand-written parser speed for comparison; whether clause-level opaque fallbacks stay at a low conflict count.

## Open questions for the orchestrator

1. Is editor highlighting a goal?
   If so, it justifies a separate tree-sitter grammar project later, but not as the formatter's parser.
2. Is input-sensitive layout acceptable, the way Topiary and partly sqlfmt work?
   If yes, Topiary's ceiling matters less.
   DESIGN.md's open question about filling long lists implies no.
3. For the hand-written parser: should the GLR-style "try known,
   else opaque per statement" be reproduced as the first fallback granularity, with finer-grained verbatim added later?

Artifacts are in `/home/glenn/.claude/jobs/ab5633ea/tmp/spike-treesitter/`:

- `scripts/grammar-{a,b}/` (spike grammars)
- `scripts/topiary/` (config and queries)
- `scripts/chfmt/` (Rust embedding)
- `scripts/samples/` (inputs and outputs)
- `dl/` (downloads)
