# Prior-art addendum: minijinja as tag finder, chum's pipeline, release tags (2026-11-10)

Read in source unless marked *(recall)*.
M = `~/.cargo/registry/src/*/minijinja-3.1.0`, C = `/home/glenn/devel/glennib/chum`.

## A1. minijinja 3.1 lexer (M/src/compiler/lexer.rs, tokens.rs, lib.rs:254-295)

- **API.** There is no `machinery.rs`.
  `pub mod machinery` sits in lib.rs behind `unstable_machinery`, documented as having "no semver guarantees".
  It exposes `Tokenizer`, `tokenize(input, in_expr, SyntaxConfig)`, `Token`, `Span`
  (byte offsets as u32), `parse` and `ast`.
- **Tag start.**
  Any `{` followed by `{`, `%` or `#`, plus an optional `-` or `+`.
  The scan has no SQL knowledge, so `'{{ x }}'` inside a SQL string is a tag.
  That is correct, because minijinja renders it.
- **Tag end.**
  The interior is lexed as Jinja: strings are escape-aware and brackets are balanced,
  so `{{ "%}" }}` and `{{ {'a': {'b': 1}} }}` end in the right place.
  `-}}`, `+}}`, `-%}` and `+%}` are recognised.
- **Comments.**
  A comment ends at the first `#}`, and no token is emitted for it.
- **Raw blocks.**
  The content becomes one `TemplateData` token.
  The `raw`/`endraw` tags themselves emit nothing.
- **Line statements.**
  Off unless a prefix is configured, and chum configures none.
- **Lossy for a formatter.**
  - Whitespace stripped by `-` is left out of the spans.
  - The final `\n` is dropped unless `keep_trailing_newline` is set.
  - The first syntax error inside any tag stops tokenizing, e.g. an unterminated string, a `?`,
    or non-ASCII when the `unicode` feature is off.
  - Its tag boundaries are reliable, but rebuilding a lossless stream means re-scanning every gap between spans.

## A2. chum (C/src/template.rs, split.rs, migrator.rs, migration.rs, source.rs, Cargo.toml)

- **Environment** (template.rs:94-112).
  - Syntax: default delimiters, `trim_blocks(false)`, `lstrip_blocks(false)`, and **`keep_trailing_newline(true)`**.
  - Strict undefined, and no auto-escaping.
  - Filters: `sql_string` (`'…'`, backslash-escaping `\` and `'`), `sql_ident`
    (backticks, ``` `` ``` doubled, `\` escaped), `sql_date`, `sql_datetime`, and `sql_datetime64(p)`.
    The last three emit `toDate…('…'[, p], 'UTC')`.
- **minijinja dependency.**
  `"~3.1.0"` with only the `builtins`, `macros`, `debug` and **`unstable_machinery`** features.
  `multi_template` is off, so there is no include.
  The tilde pin exists because `chum check` walks the compiled `Instruction`s.
- **Order: render, then split** (migrator.rs:825).
  The splitter never sees Jinja, so tags between statements or a `{% for %}`
  that emits several statements don't concern it. split.rs:
  - runs sqlparser 0.63's tokenizer with `ClickHouseDialect`;
  - cuts at top-level `SemiColon` tokens, so a `;` inside a string, comment or quoted identifier is not a cut;
  - slices the rendered text verbatim and drops leading `--` lines and empty chunks.
- **Template detection is by file extension only** (`.sql.j2` / `.j2.sql`).
  In a plain `.sql` file, `{{` is literal SQL.
- **Checksum** (migration.rs:58-64, 173-195).
  SHA-384 of the *source* text, template text included, normalised only for CRLF and trailing whitespace.
  **Reformatting an applied migration is checksum drift**, and there is no override (migrator.rs:435).

**Compatibility constraints for chairlift:**

- whitespace stays exactly as written;
- the lexer accepts `\'` in strings and ``` `` ``` in identifiers;
- Jinja handling is chosen by file extension;
- either chairlift skips applied migrations, or chum gains a checksum that ignores formatting,
  e.g. one taken over the token stream.

**Reuse:**

- `mod template` is private.
- To check that formatted output still compiles, a plain `Environment` with the same `SyntaxConfig` is enough,
  because filters are resolved at run time.
- chairlift's lexer could later replace sqlparser in split.rs.

**Recommendation: hand-roll the tag scanner in chairlift's lexer.**

- The scanner finds a `{` followed by `{`, `%` or `#`, plus an optional `-`/`+`,
  and scans the interior with Jinja string and bracket awareness.
  Comments end at the first `#}`, and `raw` blocks are opaque.
- minijinja serves as a dev-dependency oracle in a differential test that checks the tag spans across the corpus.
- **Deciding reason:** chairlift needs a lossless,
  error-tolerant byte stream. minijinja's tokenizer is built for rendering: it drops comments,
  raw delimiters and stripped whitespace, and it stops at the first error.
  Using it would also tie chairlift to an unstable API that chum already pins to `~3.1.0`,
  so the two repos would have to move that pin in lockstep.

## B. Release tags

- **release-plz** (<https://release-plz.dev/docs/config>).
  `git_tag_name` and `git_release_name` default to:
  - `"v{{ version }}"` for "a single crate or workspaces containing just one public package";
  - `"{{ package }}-v{{ version }}"` for "more than one public package".
  - The docs don't define "public" *(recall: not `publish = false`)*.
  - Both settings can be overridden per workspace or per package, using `{{ package }}` and `{{ version }}`.
- **chum's release-plz.toml** sets only `release_always = false` and `git_release_draft = true`.
  A second public crate would therefore change chum's tags to `chum-v0.6.0`.
- **cargo-dist 0.34.**
  Its release.yml runs on `push: tags: '**[0-9]+.[0-9]+.[0-9]+*'` and undrafts release-plz's draft
  (`create-release = false`).
  How it reads a tag (<https://axodotdev.github.io/cargo-dist/book/workspaces/workspace-guide.html>):
  - `v1.0.0` announces every dist-able package at that version.
  - `pkg-v1.0.0` or `pkg/1.0.0` announces only that package.
  - Library-only packages are ignored.
- **Verdict:** the DESIGN.md claim is verified.
  `chum-v0.6.0` still triggers the workflow and announces only chum, so the workflow needs no change.
  Anything that matches `v*` tags would change, such as install docs and links.
- **Not checked:** release-plz behaviour across a change of tag format in `git_only` mode.

**Implication:** moving into chum's workspace later costs little on the release side.
Accept `chum-vX`/`chairlift-vX` tags, which cargo-dist already handles,
or pin `git_tag_name = "v{{ version }}"` for chum alone.
