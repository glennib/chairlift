# Test fixtures

Inputs for chairlift's tests and benchmarks.
Files here are never reformatted by hand: a fixture is either an input as someone wrote it or an expected output.

## `design/`

- `page_views_1h.sql.j2` is the example from `docs/DESIGN.md`, section "Layout: a starting point".
- `page_views_1h.expected.sql.j2` is how chairlift should print it, from `docs/spikes/2026-11-10-layout-proposal.md`,
  section 4.
  It is the golden expectation for the layout rules (`tests/format_golden.rs`).

Both contain Jinja tags, hence the `.sql.j2` extension.

## `chum/`

Migrations copied unchanged from [chum](https://github.com/glennib/chum)'s `tests/` directory at commit
`60ce2a04358b242f93009c20d15730d5cc698cfd`, chum's version 0.5.0.
The directories are `migrations`, `template_migrations`, `bootstrap_migrations`,
`upgrade_migrations` and `question_mark_migrations`.

chum is dual-licensed under MIT or Apache-2.0, the same terms as chairlift.

## `statements/`

Inputs written for chairlift's tests, one file per family of statements beyond `SELECT` and `CREATE`:
`INSERT` with its data, and the statements of a line or two (`DROP`, `TRUNCATE`, `DETACH`, `ATTACH`, `UNDROP`,
`RENAME`, `EXCHANGE`, `OPTIMIZE`, `SET`, `USE`), `ALTER` with the lightweight `DELETE` and `UPDATE`,
`CREATE DATABASE`, `VIEW`, `DICTIONARY` and `FUNCTION`, access control (`GRANT`, `REVOKE`, users, roles,
quotas, row policies, settings profiles), and `SHOW` and `DESCRIBE`.
They are written loosely on purpose, in mixed case and spacing, so that their snapshots show the layout.

## `fmt_off/`

Inputs written for chairlift's tests, with `fmt: off` and `fmt: on` markers in awkward places,
so that the snapshots and the oracle (`corpus:oracle`, and `tests/corpus_tool.rs` over the fixtures) see them.
The code outside the regions is written loosely, as in `statements/`, and the regions are hand-aligned,
so a snapshot shows the one laid out and the other kept.

- `select.sql`: a region between statements, and inside a `SELECT` list, a `CASE` and an `IN` list.
- `create_table.sql`: a hand-aligned column list of `CREATE TABLE`.
- `values.sql`: the rows of `INSERT … VALUES`, a whole `INSERT`, and the rows of the `values()` table function.
- `spanning.sql`: a region across a statement boundary, and markers at the end of lines of code.
- `spellings.sql`: `#` markers and mixed case, comments that only look like markers,
  marker text in strings and block comments, a `fmt: on` without a `fmt: off`, and a nested `fmt: off`.
- `unmatched_off.sql`: a `fmt: off` without a `fmt: on`, which keeps the rest of the file.
- `dbt_model.sql.j2`: `{# fmt: off #}` regions after a dbt `config()` header, inside an `{% if %}` and around one.
- `migration.sql.j2`: `--` markers around a block in a template, a region around an `INSERT`,
  and Jinja comment markers at the end of lines of code.
