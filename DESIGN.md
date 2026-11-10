# chairlift: design notes

Status: initial sketch, 2026-11-10.
Everything below is a suggestion to test, not a decision.
Facts marked *(verified)* were checked against ClickHouse 26.10 on 2026-11-10;
facts marked *(unverified)* are from memory and need checking.

## What chairlift is for

chairlift is a formatter for ClickHouse SQL, including SQL files that contain Jinja templates.
Its first user is chum, a ClickHouse migration tool whose migrations are plain `.sql` files or minijinja templates
(`.up.sql.j2` / `.up.j2.sql`).
Projects built with dbt-clickhouse (Jinja plus ClickHouse SQL) have the same need.

Desired properties:

- **Opinionated**: one output style, no configuration beyond perhaps line width.
- **Fast**: a single static binary, quick enough for format-on-save and for pre-commit over a whole repository.
- **Safe**: never changes what the SQL means, and checks that on every file it writes.
- **Lossless**: every comment stays, in place.
- **Tolerant**: syntax it doesn't know is passed through, never a reason to fail the file.

## Why a new tool

Existing formatters, run on 2026-11-10 against a real migration set
(comment-heavy `CREATE TABLE`, a materialized view with a Jinja `{% if %}` block, `GRANT` and role statements):

| Tool | Result |
| --- | --- |
| sqruff 0.44 (Rust) | ~0.1 s per file, two-line config. Its ClickHouse grammar rejects `SETTINGS` after `TTL`, `ALTER USER … DEFAULT ROLE` and `DROP ROLE`, and it then leaves the file unchanged. |
| sqlfluff 4.2 | The same grammar gaps (sqruff is a port). Its Jinja templater stops at custom filters unless they are configured. |
| sql-formatter 15 (`-l clickhouse`) | Writes `CODEC (Delta, ZSTD (1))`, moves `TTL` onto the `ORDER BY` line, crashes on a Jinja filter. |
| sqlfmt 0.32 | Understands Jinja, but leaves most DDL alone, strips the hanging indent of wrapped comment lines and moves `{% %}` tags to column 0. |
| sleek 0.5 | Rewrites `String` to `STRING`, which ClickHouse rejects. |
| clickhouse-format 26.10 | Parses everything, but prints a canonical form (below), drops comments inside statements and can't parse Jinja. |

Two lessons shape the rest of this document:

1. Strict-parse tools fail on the first statement their grammar doesn't know, and then do nothing for the file.
   ClickHouse's grammar is large and grows every release, so a strict grammar will always lag behind it.
2. Tools that format tokens without understanding them make changes that break ClickHouse.
   Type names and most function names are case-sensitive: `String` works but `STRING` and `string` don't,
   and `COUNT()` works but `TOSTARTOFHOUR()` doesn't *(verified)*.

## ClickHouse's own parser and formatter

- ClickHouse is C++.
  Its parser is hand-written recursive descent in `src/Parsers/` with a separate lexer *(unverified)*.
  Comments are skipped before parsing, so they never reach the AST.
- `clickhouse-format` prints the AST with the code ClickHouse uses for canonical query text,
  such as `SHOW CREATE TABLE` output *(unverified)*.
  Its output is a normal form: it backquotes identifiers and rewrites `INTERVAL 24 HOUR` to `toIntervalHour(24)`,
  `a ? b : c` to `if(a, b, c)` and `$$…$$` to `'…'`.
  It puts `SETTINGS` on one line, and `--comments` keeps comments between statements
  but drops those inside one *(verified)*.
- The AST has no stable public form; `EXPLAIN AST` is debug output.
  Building on it would mean linking ClickHouse's C++ code, which takes hours to build and changes between releases.
- It works well as an oracle.
  `clickhouse-format -n --oneline` gives a one-line canonical form,
  and if input and output canonicalize to the same text, formatting didn't change the meaning.
  This flagged sleek's `String` → `STRING` and passed the other tools' layout-only changes *(verified)*.
  ClickHouse's debug builds reportedly check that formatting an AST
  and parsing it again gives the same AST *(unverified)*, which is what makes the canonical form trustworthy.

Suggestion: use ClickHouse as a test oracle and as an optional runtime check, not as a dependency.

## Core idea: change only whitespace and keyword case

Suggestion: chairlift changes only whitespace (indentation, line breaks, blank lines) and the case of keywords.
It never adds, removes, reorders or rewrites tokens: no quoting changes, no `<>` → `!=`,
no added semicolons or parentheses.

That allows a cheap safety check that needs no ClickHouse: lex the input and the output and compare the token streams,
ignoring whitespace.
Equal streams mean the same SQL, comments included.
Lexing the output again also catches the subtle breaks, such as `-` and `-1` joining into a `--` comment,
or a line comment losing its newline.
Black runs a similar equivalence check before writing a file; chairlift's could be cheap enough to run on every file,
every time.

Several choices that look like more than layout are still whitespace-only, so they remain open:
leading or trailing `AND`, leading or trailing commas, where lines break.

Canonical keyword case (`select` and `SELECT` both become one of the two) needs more care than it seems.
ClickHouse accepts many keywords as identifiers
(`key`, `value`, `date` and `order` are valid column names *(verified)*),
and identifiers, type names and most function names are case-sensitive.
So only words the parser has identified as keywords are re-cased.
Function, type, engine and setting names, and everything in a verbatim region, keep the case they were written in.
A file chairlift only partly understands can therefore come out with mixed keyword case;
that is the price of never guessing.

The token check then allows a word to differ in case only where the formatter re-cased a keyword,
which trusts the parser's classification.
The ClickHouse oracle covers that gap: its canonical form uppercases keywords but keeps identifiers as written,
so `select key` and `SELECT key` canonicalize the same while `SELECT KEY` doesn't *(verified)*.

## Suggested architecture

Rust, as a library crate and a CLI: a single static binary, fast, a good ecosystem for this kind of tool, and chum
(also Rust) can embed the library.

A pipeline in the style of rust-analyzer, biome and taplo:

1. **Lexer** that turns every byte into a token, whitespace and comments included,
   so the tokens concatenate back to the input exactly.
2. **Tolerant parser** that builds a lossless concrete syntax tree (for example with `rowan`).
   Statements and constructs it doesn't know become verbatim nodes,
   kept byte for byte or at most re-indented as a block.
3. **Formatting IR**: a Wadler/Prettier-style document
   (groups, indents, soft and hard line breaks) and a width-aware printer.
   Candidates: the `pretty` crate, biome's `biome_formatter` if it is usable on its own, or a small printer of our own.
4. **Verifier**: the token-stream comparison, on every run.

Worth a time-boxed spike before choosing:

- A tree-sitter grammar with [Topiary](https://github.com/tweag/topiary), which formats from tree-sitter queries.
  It would bring editor highlighting along;
  the risks are grammar conflicts at ClickHouse's size and handling Jinja inside tree-sitter.
- sqlparser-rs, whose tokenizer chum uses.
  Its AST drops comments and whitespace and its ClickHouse coverage is partial, so it probably can't be the parser,
  but its tokenizer is a useful reference.

Prior art not yet evaluated: sqlglot
(Python; has a ClickHouse dialect and regenerates SQL from its AST),
AfterShip's `clickhouse-sql-parser` (Go),
and an ANTLR grammar ClickHouse once carried under `utils/antlr/` *(unverified)*, probably stale but a useful checklist.

## Lexing details that matter

- Comments: `--`, `/* */` and `#` (ClickHouse accepts `#` *(verified)*).
- Strings with backslash escapes, backquoted and double-quoted identifiers, heredocs
  (`$$…$$`, `$tag$…$tag$`), numbers such as `0x1F`, `1e3`, `inf` and `nan`.
- Query parameters, `{name:Type}`, which `clickhouse-format` preserves *(verified)* and which use braces like Jinja.
- `INSERT … FORMAT <name>` followed by inline data: everything after the format name is data and stays verbatim.
- Jinja tags (next section).

## Jinja

Treat Jinja as the outer language: find the Jinja tags first, then lex SQL in the text between them.
For reference, chum renders with minijinja using the default delimiters, `trim_blocks` and `lstrip_blocks` off,
strict undefined variables, and SQL-quoting filters
(`sql_string`, `sql_ident`, `sql_date`, `sql_datetime`, `sql_datetime64(n)`).

A starting point close to what sqlfmt does:

- `{{ … }}` is an opaque atom wherever an expression or identifier can stand.
  A tag glued to identifier characters (`events_{{ suffix }}`) is part of one identifier-like token.
- `{% … %}` block tags go on their own line, indented to the level of the SQL they wrap.
  The parser sees the SQL of all branches at once.
  When that doesn't parse, for example `{% if %}…{% else %}…{% endif %}` choosing between two whole clauses,
  the region falls back to verbatim.
- `{# … #}` is a comment.
- Nothing inside a tag is formatted, and a tag inside a SQL string literal stays part of the string.

Whitespace control (`{%-`, `-%}`) strips whitespace next to a tag at render time,
so moving a tag across a line break can join two SQL tokens in the rendered output.
The token check on the template source can't see that; the render check below can.

## Verification and tests

- On every run: token-stream equality and idempotence (`fmt(fmt(x)) == fmt(x)`).
- In tests, and behind an optional `--verify` flag: equal `clickhouse-format -n --oneline` output,
  for plain SQL directly and for templates after rendering them with representative values
  (each `{% if %}` both ways), since `clickhouse-format` can't parse unrendered Jinja *(verified)*.
- Snapshot tests per fixture.
- Corpora: chum's `tests/migrations/`
  (deliberately tricky DDL);
  ClickHouse's `tests/queries/0_stateless/`, thousands of real statements
  (skip what `clickhouse-format` rejects, since some are negative tests);
  ClickHouse's documentation examples; private migration sets run locally.
- Fuzzing for no panics, token equality and idempotence.
- A benchmark over the `0_stateless` corpus, so speed stays a tested property.

## Layout: a starting point

ClickHouse users see `clickhouse-format`'s layout daily in `SHOW CREATE TABLE`, so it is a reasonable baseline:
one clause per line, `(` on its own line before a column list, four-space indents.
Depart from it where it reads badly, as with one-line `SETTINGS` and backquotes everywhere.
Other suggestions:

- Keep a single blank line where the author put one
  (after a header comment, between groups of columns), and collapse runs of blank lines.
- A comment on its own line attaches to the next node; a trailing comment stays on its line.
- Never reflow comment text.
- Leading `AND`/`OR` in long conditions, which suits `WHERE true` followed by Jinja-guarded conditions.

An input that covers most of the hard cases:

```sql
-- Hourly page views per site. The blank line after this header is
-- intentional.

create table analytics.page_views_1h
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

create materialized view analytics.page_views_1h_mv to analytics.page_views_1h as
select
    toStartOfHour(event_time) as hour,
    site,
    count() as views,
    uniqState(visitor_id) as visitors
from raw.events
where true
    and event_type = 'page_view'
    {% if this.cutoff is defined -%}
    and event_time >= {{ this.cutoff | sql_datetime }}
    {% endif -%}
group by hour, site;

GRANT SELECT ON analytics.page_views_1h TO {{ this.reader_role | sql_ident }};
```

## Interfaces

- `chairlift [PATH…]` formats in place; `--check` lists unformatted files and exits non-zero; `--diff` prints a diff;
  `-` reads stdin and writes stdout, for editors.
- File discovery: `*.sql`, `*.sql.j2` and `*.j2.sql`, respecting `.gitignore`.
- Library: something like `format(source, options) -> Result<String, Error>`,
  so chum could offer `chum fmt` or check formatting when it loads migrations.

## Where the code lives

Either of these works:

- **Its own repository** (this one), with chum depending on the published crate.
  The formatter gets its own release cadence and issue tracker, and its early churn stays out of chum's history.
  A change that touches both tools, such as sharing a lexer, takes two releases.
- **A virtual Cargo workspace in chum's repository**, with the migrator and the formatter as separate crates.
  Changes that touch both land in one commit and share CI, the `tests/migrations/` corpus and the release tooling.
  The cost is the move itself: chum's tasks and docs assume a single crate,
  and once a workspace publishes more than one package release-plz switches to per-package tags
  (`chum-v0.6.0` rather than `v0.6.0`) *(unverified)*, which the cargo-dist release workflow is triggered by.

The lexer is the strongest reason to share code: chum finds statement boundaries with sqlparser's tokenizer,
and a ClickHouse-aware lexer could serve both tools.
One path is to start here and move into a workspace with chum once chum depends on chairlift.

## Out of scope at first

Lint rules, other SQL dialects, semantic checks
(types, whether tables exist), formatting inside Jinja tags, reflowing comment prose, a language server.

## Open questions

- Keywords upper or lower case?
  Upper matches `clickhouse-format` and the ClickHouse documentation.
- How far should Jinja support go: opaque tags, or `if`/`for` blocks understood as structure?
- Hand-written parser or tree-sitter?
  A spike of each on `CREATE TABLE` plus a materialized view could decide it.
- Line width (80? 100?), and whether it is configurable at all.
- Long lists (`IN (…)`, array literals, `SETTINGS`): fill lines, or one item per line?
- Is the token check cheap enough never to need an off switch?
  Measure it.

## A possible first milestone

1. A lexer for SQL and Jinja, with a round-trip test over the corpora (tokens concatenate back to the input).
2. The token-stream verifier.
3. A CLI with `--check` whose only formatting is safe cleanup: trailing whitespace, runs of blank lines,
   the final newline.
4. Then statement by statement, starting with `CREATE TABLE`, `CREATE MATERIALIZED VIEW … AS SELECT` and `SELECT`,
   with everything else verbatim.
