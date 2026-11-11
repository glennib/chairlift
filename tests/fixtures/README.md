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
`INSERT` with its data.
They are written loosely on purpose, in mixed case and spacing, so that their snapshots show the layout.
