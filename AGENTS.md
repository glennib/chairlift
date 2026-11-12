# Agent guide

chairlift is a formatter for ClickHouse SQL and Jinja-templated SQL: one Rust crate, a library and a CLI, `chl`.
This file holds what an agent needs before its first edit and cannot get quickly from the code.
Keeping it current is part of every task; see [Keeping this file current](#keeping-this-file-current).

## The invariant

Formatting changes only whitespace and the case of keywords.
No token is added, removed, reordered or rewritten, and syntax the parser does not understand is kept as written.
A change that makes the verifier or the oracle fail is wrong, however good the output looks;
never weaken `src/verify.rs` or add to `tests/oracle-allowlist.txt` to make a layout change pass.
The allowlist is for documented quirks of `clickhouse-format` only.

## Where things are

| What | Where |
| --- | --- |
| Style, CLI and library contract | `README.md` |
| Purpose and architecture | `docs/DESIGN.md` |
| Decisions and acceptance criteria; wins over `DESIGN.md` where they disagree | `docs/MILESTONES.md` |
| Current state, loose ends, questions waiting for the project owner | `docs/PROGRESS.md` |
| How to write grammar | rustdoc at the top of `src/syntax/grammar/mod.rs` |
| How to write layout rules | rustdoc at the top of `src/layout/mod.rs` |
| Where each vendored fixture comes from | `tests/fixtures/README.md` |

The pipeline, in the order a text passes through it: `src/lexer`, `src/split.rs`, `src/syntax`, `src/layout`, `src/doc`,
`src/verify.rs`.
`src/oracle` runs `clickhouse-format`, and `src/template.rs` renders templates for the tests.

## Commands

Tools are pinned in `mise.toml`, and every check is a mise task.
Run `mise tasks` for the list instead of trusting a copy of it here.

- `mise install` once, then `mise run ci` before calling work done: it is what CI runs.
- `mise run test:doc` runs the doctests, which nextest does not.
- `mise run corpus:fetch` fetches the ClickHouse and dbt corpora into `$CHAIRLIFT_CORPUS_DIR`, by default `corpus/`.
  Tests that need a fetched corpus or `clickhouse-format` skip silently without them,
  so a green `mise run test` does not cover them; `mise run test:corpus` fails instead of skipping.
- A change to the lexer, grammar or layout also needs `mise run corpus:oracle` and `mise run test:keyword-positions`,
  which CI runs only weekly.
- `mise run tree -- FILE` prints a file's syntax tree.
- The fuzz tasks need the nightly toolchain.

## Rules that are easy to break

- The library builds with no features; `cli`, `oracle`, `template`, `corpus-tool` and `unstable` are optional.
  Put a new dependency behind the feature that needs it; `mise run clippy:lib` checks each feature alone.
- `src/keyword/generated.rs` is generated: change it with `mise run keywords:generate`, not by hand.
- The ClickHouse version is pinned in three places that move together: `clickhouse` in `mise.toml`,
  `CLICKHOUSE_VERSION` in `src/lib.rs`, and the tag in `scripts/fetch-corpus.sh`.
- The published crate holds only what `include` in `Cargo.toml` lists.
  A unit test that reads a new file needs the file listed there; `mise run package:test` checks it.
- Snapshot tests use insta.
  A changed snapshot is a change of style or of the tree: show the diff and say why, do not accept it in passing.
  A failed inline snapshot leaves a `tests/.*.pending-snap` file that nothing ignores: delete it before committing.
- Private corpora (`CHAIRLIFT_LOCAL_CORPORA`) are never vendored, copied into fixtures or quoted in tests.
- A user-visible change of style or of the CLI goes into `README.md` in the same commit.
- Commit titles follow Conventional Commits
  (`feat(layout): …`, `fix(corpus): …`); release-plz builds the changelog from them.

## Keeping this file current

Before finishing a task, reread this file against what the task changed and taught, and edit it in the same commit.

Update it when:

- a path, task, feature or command named here was renamed, moved or removed;
- you lost time to something that a line here would have prevented, and the next agent would lose it too;
- the project owner corrected your approach in a way that applies beyond the task at hand;
- a rule here turned out to be wrong or no longer has a reason: delete it.

Keep it out when:

- it is in the code, in `mise tasks`, or in one of the documents in the table above: link to it instead;
- it is status, a plan, or a record of what was done: that belongs in `docs/PROGRESS.md` or the commit message;
- it is one person's preference or one machine's setup.

Keep the file under about 100 lines.
When adding a line would pass that, remove or merge a less useful one first.
Every path and task named here must exist: check with `mise tasks` and `ls` when editing.
