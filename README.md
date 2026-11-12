# chairlift

chairlift is a formatter for ClickHouse SQL, including SQL files that contain Jinja templates,
such as dbt-clickhouse models and [chum](https://github.com/glennib/chum) migrations.
It is a single binary, `chl`, and a Rust library.

It changes only whitespace and the case of keywords.
It never adds, removes, reorders or rewrites a token,
and it checks that on every run by comparing the tokens of its output with those of its input.
Syntax it does not understand is kept as written instead of failing the file,
and every comment stays where its author put it.

```console
$ echo 'select a, b from t where x = 1 and y = 2' | chl -
SELECT
    a,
    b
FROM t
WHERE x = 1 AND y = 2
```

What it does not do: lint, rewrite queries, format the code inside Jinja tags, reflow comments,
or understand SQL dialects other than ClickHouse's.
It never changes string or number literals, comments, the data of an `INSERT`, or the case of names: identifiers,
aliases, and function, type, engine and setting names.

## Installation

No release has been published yet.
The commands in this section work from the first release, 0.1.0, on.

Prebuilt binaries for Linux (static, musl), macOS and Windows, x86-64 and ARM64
(ARM64 on Linux and macOS only), are attached to each [GitHub release](https://github.com/glennib/chairlift/releases).
The installer scripts download the right one:

```shell
# Linux and macOS
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/glennib/chairlift/releases/latest/download/chairlift-installer.sh | sh
```

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/glennib/chairlift/releases/latest/download/chairlift-installer.ps1 | iex"
```

With a Rust toolchain, from crates.io:

```shell
cargo install --locked chairlift
```

`cargo binstall chairlift` looks for a prebuilt binary in the GitHub release before it builds from source.

Before the first release, build from the repository:

```shell
cargo install --locked --git https://github.com/glennib/chairlift chairlift
```

`chl --version` prints chairlift's version and the ClickHouse version whose keyword list it was built against.

## Usage

```text
chl [OPTIONS] <PATHS>...
```

Paths are files or directories, formatted in place.
In a directory chairlift formats `*.sql`, `*.sql.j2` and `*.j2.sql` files, skipping hidden files,
symbolic links and whatever `.gitignore` and `.ignore` files exclude.
A file named on the command line is formatted whatever its name.
`-` reads stdin and writes the result to stdout.

```console
$ chl .
1 file reformatted, 2 files already formatted
```

`--check` writes nothing, lists the files that would change, and exits with 1 if there are any;
`--diff` prints the changes as a unified diff instead:

```console
$ chl --check .
Would reformat: ./migrations/0001_init.sql
1 file would be reformatted, 2 files already formatted
$ chl --diff migrations
--- a/migrations/0001_init.sql
+++ b/migrations/0001_init.sql
@@ -1 +1,5 @@
-select a, b from t where x = 1 and y = 2;
+SELECT
+    a,
+    b
+FROM t
+WHERE x = 1 AND y = 2;
1 file would be reformatted, 1 file already formatted
```

The options:

| Option | What it does |
| --- | --- |
| `--check` | Write nothing; list the files that would change, exit 1 if any would |
| `--diff` | Write nothing; print a unified diff, exit 1 if any file would change |
| `--line-width <WIDTH>` | The width lines are laid out to fit; 100 by default |
| `--jinja <auto\|on\|off>` | Whether files are Jinja templates (see [Jinja templates](#jinja-templates)); `auto` by default |
| `--stdin-filename <PATH>` | The name of the text read from stdin, for `--jinja auto` and for messages; the file is not read |
| `-j`, `--jobs <N>` | Files formatted at once; the number of CPUs by default |
| `-q`, `--quiet` | Print only errors: no summary or warnings |
| `-v`, `--verbose` | Also print what happened to each file, and how long it took |
| `--color <auto\|always\|never>` | When to colour diagnostics; `auto` colours them on a terminal unless `NO_COLOR` is set |

The line width and the Jinja mode are the only settings of the style, and there is no configuration file.

### Exit status

| Status | When |
| --- | --- |
| 0 | Done: files formatted, or nothing to change |
| 1 | `--check` or `--diff` found files that would change |
| 2 | A file could not be formatted (unreadable, not UTF-8, a lexer error, a failed verification, or a refused template), or the arguments are wrong |

A file that cannot be formatted is left as it is, and a diagnostic on stderr says why:

```console
$ printf "select 'unclosed from t\n" | chl --stdin-filename query.sql -
chairlift::lex

  × query.sql:1:8: Single quoted string is not closed; the file is left unchanged
   ╭─[query.sql:1:8]
 1 │ select 'unclosed from t
   ·        ┬
   ·        ╰── here
   ╰────
```

## Style

In short:

- Keywords in upper case, but only where the parser read a keyword: ClickHouse has few reserved words,
  so a column named `date` or `key` keeps its case.
  `true`, `false`, `inf` and `nan` are written in lower case and `NULL` in upper case,
  as `clickhouse-format` prints them.
- Lines up to 100 characters, or `--line-width`.
- A list stays on one line if it fits, and otherwise goes one item per line, with trailing commas.
  A `SELECT` list with more than one item always goes one item per line.
- Conditions hang: the first stays on the `WHERE` line, and each further one starts its line with `AND` or `OR`.
- Subqueries and a single CTE hug their brackets.
  The column list of `CREATE TABLE` puts `(` on a line of its own, as `SHOW CREATE TABLE` prints it,
  and columns are not aligned.
- Blank lines are kept as written, within limits: at most two between statements and one inside a statement,
  and always one around a statement that spans lines.
- Comments are never reflowed.
  A comment on its own line takes the indentation of what follows it, and a trailing comment stays on its line.

The full style sheet, with the alternatives that were considered,
is [`docs/spikes/2026-11-10-layout-proposal.md`](docs/spikes/2026-11-10-layout-proposal.md).
The examples below are chairlift's output for the input above each.

A query with a CTE, joins and a subquery:

```sql
with recent as (select user_id, max(ts) as last_seen from events where ts > now() - interval 7 day group by user_id)
select u.id, u.name, r.last_seen, count() as n from users u inner join recent r on r.user_id = u.id and u.active = 1 left join orders o on o.user_id = u.id where u.country in (select code from countries where region = 'EU') group by u.id, u.name, r.last_seen order by n desc limit 10
```

```sql
WITH recent AS (
    SELECT
        user_id,
        max(ts) AS last_seen
    FROM events
    WHERE ts > now() - INTERVAL 7 DAY
    GROUP BY user_id
)
SELECT
    u.id,
    u.name,
    r.last_seen,
    count() AS n
FROM users u
INNER JOIN recent r ON r.user_id = u.id AND u.active = 1
LEFT JOIN orders o ON o.user_id = u.id
WHERE u.country IN (
    SELECT code
    FROM countries
    WHERE region = 'EU'
)
GROUP BY u.id, u.name, r.last_seen
ORDER BY n DESC
LIMIT 10
```

Conditions that do not fit on one line:

```sql
select * from t where (a = 1 or b = 2) and c in (1, 2, 3) and d between 1 and 10 and e like '%x%' and f is not null and g = 'long enough to break the line'
```

```sql
SELECT *
FROM t
WHERE (a = 1 OR b = 2)
    AND c IN (1, 2, 3)
    AND d BETWEEN 1 AND 10
    AND e LIKE '%x%'
    AND f IS NOT NULL
    AND g = 'long enough to break the line'
```

A table, a materialized view, an `INSERT`, an `ALTER` and a `GRANT`:

```sql
create table if not exists events (ts DateTime64(3) codec(Delta, ZSTD), user_id UInt64, kind LowCardinality(String), payload String default '' comment 'raw JSON') engine = MergeTree partition by toYYYYMM(ts) order by (user_id, ts) ttl ts + interval 90 day settings index_granularity = 8192;

create materialized view daily_mv to daily as select toDate(ts) as day, kind, count() as n from events group by day, kind;

insert into daily (day, kind, n) select toDate(ts), kind, count() from events where ts >= today() group by 1, 2;

alter table events add column if not exists country LowCardinality(String) after user_id, modify ttl ts + interval 30 day;

grant select(id, name, country), insert on analytics.users to reporting_role, etl_user, dashboard_reader with grant option;
```

```sql
CREATE TABLE IF NOT EXISTS events
(
    ts DateTime64(3) CODEC(Delta, ZSTD),
    user_id UInt64,
    kind LowCardinality(String),
    payload String DEFAULT '' COMMENT 'raw JSON'
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(ts)
ORDER BY (user_id, ts)
TTL ts + INTERVAL 90 DAY
SETTINGS index_granularity = 8192;

CREATE MATERIALIZED VIEW daily_mv TO daily AS
SELECT
    toDate(ts) AS day,
    kind,
    count() AS n
FROM events
GROUP BY day, kind;

INSERT INTO daily (day, kind, n)
SELECT
    toDate(ts),
    kind,
    count()
FROM events
WHERE ts >= today()
GROUP BY 1, 2;

ALTER TABLE events
    ADD COLUMN IF NOT EXISTS country LowCardinality(String) AFTER user_id,
    MODIFY TTL ts + INTERVAL 30 DAY;

GRANT SELECT(id, name, country), INSERT
    ON analytics.users
    TO reporting_role, etl_user, dashboard_reader
    WITH GRANT OPTION;
```

A dbt model, formatted with `--jinja on`:

```sql
{{ config(materialized='incremental', order_by='(user_id, day)') }}

select user_id, toDate(ts) as day, count() as events
from {{ source('raw', 'events') }}
where kind != 'heartbeat'
{% if is_incremental() %}
and ts >= today() - {{ var('lookback_days', 3) }}
{% endif %}
group by user_id, day
```

```sql
{{ config(materialized='incremental', order_by='(user_id, day)') }}

SELECT
    user_id,
    toDate(ts) AS day,
    count() AS events
FROM {{ source('raw', 'events') }}
WHERE kind != 'heartbeat'
    {% if is_incremental() %}
    AND ts >= today() - {{ var('lookback_days', 3) }}
    {% endif %}
GROUP BY user_id, day
```

## Jinja templates

With `--jinja on`, chairlift reads `{{ … }}`, `{% … %}` and `{# … #}` as template tags.
`--jinja auto`, the default, treats files named `*.j2`
(such as `*.sql.j2`) and `*.j2.sql` as templates, and stdin as one only if `--stdin-filename` names a template.
A dbt model is named `*.sql`, so a dbt project needs `--jinja on`.
`--jinja off` reads every file as plain SQL; in plain SQL `{` is a ClickHouse token,
so a statement holding a template tag does not parse and is kept as written.

How templates are laid out:

- `{{ … }}` stands for an expression or a name, and is never changed inside; `events_{{ suffix }}` is one word.
  Whether there is whitespace next to a `{{ … }}` is kept as written, since the tag can render to anything.
- A block tag (`{% if %}`, `{% for %}` and the rest) goes on a line of its own at the indentation of the SQL it wraps,
  and `{% else %}`, `{% elif %}`, `{% endif %}` and `{% endfor %}` line up with their opener.
  Between statements, block tags start at column 0.
- The parser steps over block tags, so it sees the SQL of every branch at once.
- Text inside tags is never changed, including the `-` and `+` whitespace-control markers.
- A tag inside a string literal is part of the string.

Where the layout cannot be sure that every render of the template keeps its meaning, the statement is kept as written.
Examples are a tag that stands for a keyword
(`{% if f %}where{% else %}and{% endif %}`), and statements in which rendering could join two tokens into one.

A template in which a line comment is followed by a tag with a `-` marker is refused:
where that comment ends depends on the rendering, so moving any line break could change what it comments out.
The file is left unchanged, and chairlift exits with 2:

```console
$ chl refused.sql.j2
chairlift::template

  × refused.sql.j2:1:10: where this line comment ends depends on the rendering, so the file is left unchanged
   ╭─[refused.sql.j2:1:10]
 1 │ select a -- first column
   ·          ┬
   ·          ╰── this comment
 2 │ {%- if wide %}, b{% endif %}
   ╰────
  help: the `-` marker of the tag after it strips the line break after it, and the comment can run on into text that follows a tag on its line; remove the marker, or end the lines of the tags there
0 files reformatted, 0 files already formatted, 1 template refused
```

## Safety

chairlift is meant to run on migrations and models that people depend on.
Every run, including `--check` and `--diff`, checks each result before it is written or printed,
and this check cannot be turned off:

- The tokens of the output are compared with those of the input, ignoring whitespace.
  A word may differ only in case, and only where the parser read a keyword;
  comments and Jinja tags must match byte for byte.
- The output is lexed again, so that a line comment that would swallow the next line,
  or two tokens that would run together, are caught.

A result that fails the check is not written; chairlift reports it as a bug and exits with 2.
A file with a lexer error, such as an unclosed string or comment, is left unchanged and reported,
as is a file that is not UTF-8.

A statement chairlift cannot parse, or has no layout for, is kept as written:
its trailing whitespace goes and runs of blank lines are shortened, and nothing else changes.

chairlift does not need ClickHouse to run.
ClickHouse's own parser checks chairlift during development instead:
a weekly CI job runs the `clickhouse-format` of ClickHouse 26.10 on the input and the output of every statement
in the corpora, and of every rendering of every template,
and fails on any statement ClickHouse reads differently, apart from documented quirks of `clickhouse-format`.

## Editor setup

`chl -` reads a buffer on stdin and writes the formatted text to stdout.
On success it prints nothing else.
When the text cannot be formatted it exits with 2, prints nothing on stdout and the reason on stderr,
so an editor that keeps the buffer when a formatter fails loses nothing.
Pass the file's name with `--stdin-filename`, so that `--jinja auto` can decide by it and messages name the file.
For dbt models, add `--jinja on`.

Any editor that can pipe a buffer through a command can use it that way.

### Neovim

With [conform.nvim](https://github.com/stevearc/conform.nvim):

```lua
require("conform").setup({
  formatters = {
    chairlift = {
      command = "chl",
      args = { "--stdin-filename", "$FILENAME", "-" },
    },
  },
  formatters_by_ft = {
    sql = { "chairlift" },
  },
})
```

Neovim gives `*.sql.j2` files no filetype; to format them too, map them to `sql`:

```lua
vim.filetype.add({ pattern = { [".*%.sql%.j2"] = "sql" } })
```

### Helix

In `languages.toml`:

```toml
[[language]]
name = "sql"
formatter = { command = "chl", args = ["-"] }
auto-format = true
```

In a dbt project, use `args = ["--jinja", "on", "-"]`, for example in the project's `.helix/languages.toml`.

### VS Code

VS Code has no setting that runs a command as a formatter; an extension that does is needed.
With [Custom Local Formatters](https://marketplace.visualstudio.com/items?itemName=jkillian.custom-local-formatters),
in `settings.json`:

```json
{
  "customLocalFormatters.formatters": [
    {
      "command": "chl -",
      "languages": ["sql"]
    }
  ]
}
```

## pre-commit

With `chl` on `PATH`, a local hook in `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: local
    hooks:
      - id: chairlift
        name: chairlift
        entry: chl
        language: system
        files: \.sql(\.j2)?$
```

The hook formats the staged files in place, and pre-commit fails the commit when a file changed, so it can be reviewed.
To fail without changing files, set `args: [--check]`; for dbt models, `args: [--jinja, "on"]`
(quoted, because YAML reads a bare `on` as `true`).

From the first release on, the repository can also be used as a pre-commit hook repository.
Its hook uses `language: rust`, so pre-commit builds chairlift from source with cargo the first time it runs:

```yaml
repos:
  - repo: https://github.com/glennib/chairlift
    rev: v0.1.0
    hooks:
      - id: chairlift
```

pre-commit runs `cargo install` without `--locked`,
so that build takes the newest compatible version of each dependency,
not the versions in chairlift's `Cargo.lock` that the release was tested with.
The local hook with a released binary does not have that problem.

## Use as a library

The crate's default `cli` feature pulls in the command line's dependencies; a library user can leave it out:

```toml
[dependencies]
chairlift = { version = "0.1", default-features = false }
```

```rust
use chairlift::Options;

fn main() -> Result<(), chairlift::Error> {
    let options = Options::default().with_line_width(80);
    let formatted = chairlift::format("select a, b from t where x = 1;", &options)?;
    assert_eq!(formatted, "SELECT\n    a,\n    b\nFROM t\nWHERE x = 1;\n");
    assert!(chairlift::check(&formatted, &options)?);

    // Text is read as a Jinja template only when asked to.
    let template = Options::default().with_jinja(true);
    let formatted = chairlift::format("select {{ col }} from {{ ref('t') }}", &template)?;
    assert_eq!(formatted, "SELECT {{ col }}\nFROM {{ ref('t') }}\n");
    Ok(())
}
```

`format` returns an error, and no text, for input with a lexer error, a template it refuses,
or a result that failed verification.
The API documentation is on [docs.rs](https://docs.rs/chairlift).
The lexer, parser, layout and the other modules behind `format` are behind the `unstable` feature,
for chairlift's own tests and tools; they have no semver guarantee and any release may change them.

## Limitations

- `SYSTEM`, `EXPLAIN`, KQL `PRINT` and a few rare statement kinds are kept as written,
  apart from trailing whitespace and blank lines.
- A trailing comment is not counted when a line is measured, so a line ending in one can be longer than the line width.
  The width counts characters, not their display width.
- CRLF line endings become LF, except inside comments, strings, Jinja tags and `INSERT` data, which keep every byte.
- An in-place write replaces the file, so it does not keep the file's owner, extended attributes, ACLs or hard links.
- chairlift does not model what Jinja `if` and `for` blocks do to the SQL around them:
  a statement in which a block tag splits a word or an expression, as in `a{% if x %}_b{% endif %}`, is kept as written.

## Documentation

- [`docs/DESIGN.md`](docs/DESIGN.md): what chairlift is for, why a new tool, and the architecture.
- [`docs/MILESTONES.md`](docs/MILESTONES.md): the decisions taken, and the milestones with their acceptance criteria.
- [`docs/spikes/`](docs/spikes): the spike reports behind those decisions.

## Development

The tools are pinned in `mise.toml`, including ClickHouse 26.10, whose `clickhouse-format` serves as the test oracle.

```shell
mise install          # toolchain helpers and ClickHouse
mise run ci           # formatting, clippy, actionlint, shellcheck, tests, builds
mise run corpus:fetch # the ClickHouse stateless tests and the dbt samples
mise run bench        # throughput, including the stateless corpus when it is fetched
mise run fuzz:lexer -- -max_total_time=600  # fuzz the lexer; needs the nightly toolchain
```

Test corpora:

- `tests/fixtures/` holds the vendored ones; its README says where each comes from.
- `mise run corpus:fetch` clones the public ones into `$CHAIRLIFT_CORPUS_DIR`, by default `corpus/`, which git ignores.
  Point several checkouts at one directory to share a copy.
- Tests that read a fetched corpus skip when it is absent; `mise run test:corpus` runs them and fails instead.
  They include a comparison with ClickHouse's own lexer, compiled from the fetched source, which needs `clang++`
  (or the compiler named in `CHAIRLIFT_CLICKHOUSE_CXX`).
- Corpora that must stay on one machine, such as private migrations, are never fetched or vendored.
  List their directories in `CHAIRLIFT_LOCAL_CORPORA`, separated by `:`, for the corpus tooling to include;
  directories that do not exist are skipped.
  A `mise.local.toml`, which git ignores, is a convenient place to set it.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
