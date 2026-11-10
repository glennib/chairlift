# chairlift

**Pre-release: chairlift does not format anything yet.**
The repository holds the plan, the test corpora and the build and CI scaffolding;
the milestones in [`docs/MILESTONES.md`](docs/MILESTONES.md) lead to a first release.

chairlift is an opinionated formatter for ClickHouse SQL, including SQL files that contain Jinja templates,
such as [chum](https://github.com/glennib/chum) migrations and dbt-clickhouse models.
It changes only whitespace and the case of keywords, never adds, removes, reorders or rewrites a token,
and proves that on every file it writes by comparing the token streams of input and output.
Syntax it does not understand passes through unchanged instead of failing the file,
and every comment stays where its author put it.
It is written in Rust, as a library and a single static binary.

## Style

The full style sheet with rendered examples is `docs/spikes/2026-11-10-layout-proposal.md`; in short:

- Keywords in UPPER case, but only where the parser saw a keyword.
  Function, type, engine and setting names, identifiers and aliases keep their case.
  `true`, `false`, `inf` and `nan` are lowercase and `NULL` uppercase, as `clickhouse-format` prints them.
- Lines up to 100 characters, configurable with `--line-width`.
- A list stays on one line if it fits, otherwise one item per line, with trailing commas.
  A `SELECT` list with more than one item always splits.
- Conditions hang: the first stays on the `WHERE` line, and continuation lines start with `AND` or `OR`.
- No column alignment in `CREATE TABLE`; its column list keeps `(` on its own line, as `SHOW CREATE TABLE` prints it.
  Subqueries and a single CTE hug their `(`.
- Blank lines are kept as written, within limits: at most two between statements and one inside a statement,
  and always one around a statement that spans several lines.
- Comments are never reflowed; an own-line comment takes the indent of what follows it,
  and a trailing comment stays on its line.
- Jinja block tags go on their own line at the indent of the SQL they wrap; `{{ … }}` is an opaque atom;
  nothing inside a tag changes.

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
