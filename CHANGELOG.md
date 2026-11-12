# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/glennib/chairlift/compare/v0.2.0...v0.2.1) - 2026-11-12

### Fixed

- *(deps)* update rust crate similar to v3
- *(layout)* do not count a byte-order mark as indentation of text kept as written

## [0.2.0](https://github.com/glennib/chairlift/releases/tag/v0.2.0) - 2026-11-12

The first release of chairlift, a formatter for ClickHouse SQL, including SQL files that contain Jinja templates,
such as dbt-clickhouse models and [chum](https://github.com/glennib/chum) migrations.
It is a single binary, `chl`, and a Rust library.
(0.1.0 was published only to set up trusted publishing on crates.io, and is yanked.)

```console
$ chl --jinja on - < models/daily_events.sql
{{ config(materialized='incremental') }}
SELECT
    user_id,
    toDate(ts) AS day,
    count() AS events
FROM {{ source('raw', 'events') }}
WHERE kind != 'heartbeat'  -- noise
    {% if is_incremental() %}
    AND ts >= today() - {{ var('lookback_days', 3) }}
    {% endif %}
GROUP BY user_id, day
```

### Highlights

- **It changes only whitespace and the case of keywords.**
  No token is added, removed, reordered or rewritten.
  Every run, `--check` and `--diff` included, compares the tokens of the output with those of the input
  and lexes the output again before anything is written, and that check cannot be turned off.
- **Syntax it does not understand is kept as written** instead of failing the file,
  and every comment stays where its author put it.
- **It reads SQL the way ClickHouse does.**
  The lexer gives the same tokens as ClickHouse's own on 12,926 files of ClickHouse's tests,
  and the keyword list is generated from ClickHouse 26.10's parser.
  A keyword is upper-cased only where the parser read one, so a column named `date` or `key` keeps its case.
- **ClickHouse's own parser checks it.**
  `clickhouse-format` is run on the input and the output of every statement in ClickHouse's stateless tests
  and documentation examples, and of every render of every template in the corpora.
  Of the 280,333 statements it judges, it reads none differently, apart from one documented quirk of its own.
  That check runs weekly in CI, as do 15 minutes of fuzzing for each of the lexer, parser, printer and formatter.
- **Jinja templates are laid out, not skipped.**
  `{{ … }}` is a word the layout never looks inside; `{% if %}` and `{% for %}` go on lines of their own,
  indented with the SQL they wrap.
  Where a render could change what the SQL means, the statement is kept as written,
  and a template whose comments a render can run on is refused with a diagnostic that says why.
- **It covers ClickHouse's statements**: `SELECT` with CTEs, joins, `ARRAY JOIN` and subqueries, `INSERT`,
  `CREATE TABLE`, `MATERIALIZED VIEW`, `VIEW`, `DATABASE`, `DICTIONARY`, `FUNCTION` and `INDEX`,
  `ALTER` and the lightweight `DELETE` and `UPDATE`, `DROP`, `TRUNCATE`, `ATTACH`, `RENAME`, `OPTIMIZE`, `SET`,
  `USE` and their kin, `GRANT`, `REVOKE` and the users, roles, quotas, row policies and settings profiles,
  and `SHOW` and `DESCRIBE`.
- **It is fast.**
  `chl --check` goes through the 12,875 SQL files of ClickHouse's stateless tests in under a second on eight cores.

### The command line

- `chl PATH…` formats files and directories in place.
  `--check` lists the files that would change and exits with 1, and `--diff` prints a unified diff.
- In a directory it formats `*.sql`, `*.sql.j2` and `*.j2.sql`, and honours `.gitignore` and `.ignore`.
  A `.chlignore` file and `--exclude GLOB` skip files from formatting only, also when pre-commit names them.
- `chl.toml` holds the settings a project would otherwise pass on every run: `line-width`, `jinja` and `exclude`.
  The line width (100 by default) and the Jinja mode are the only settings of the style.
- `-- fmt: off` and `-- fmt: on`, sqlfmt's markers, keep the lines between them byte for byte;
  `{# fmt: off #}` and `{# fmt: on #}` work in templates.
- `chl -` formats stdin for editors, with `--stdin-filename` to name the buffer.
  The README has the setup for Neovim, Helix, VS Code and pre-commit.
- Problems are reported as diagnostics that point at the place in the file,
  and a file that cannot be formatted is left unchanged.
  Files are formatted in parallel (`-j`).

### Installing

Prebuilt binaries for Linux (glibc, and static musl), macOS and Windows are attached to this release.

```shell
mise use -g github:glennib/chairlift@latest --pin
cargo binstall chairlift
cargo install --locked chairlift
```

### As a library

`chairlift::format` and `chairlift::check` take a text and `Options`; leave out the default `cli` feature:

```toml
chairlift = { version = "0.2", default-features = false }
```

The lexer, parser and layout are behind the `unstable` feature, for chairlift's own tests and tools,
and have no semver guarantee.

### Known limitations

- `SYSTEM`, `EXPLAIN`, KQL `PRINT` and a few rare statement kinds are kept as written,
  apart from trailing whitespace and blank lines.
- A Jinja block tag that splits a word or an expression, as in `a{% if x %}_b{% endif %}`,
  and the loop-comma idiom `{{ c }}{% if not loop.last %},{% endif %}` keep their statement as written.
- A trailing comment is not counted when a line is measured, so a line ending in one can run past the line width.

The [README](https://github.com/glennib/chairlift#readme) has the full style, with before-and-after examples,
and everything above in detail.
