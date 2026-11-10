# ClickHouse parser/formatter research for chairlift

Research report, 2026-11-10.
Source: tag `v26.10.2.103-stable` (commit `c6428ebcc3`), the same version as the local binary (26.10.2.103).
The clone was shallow, blobless and sparse
(393 MB)
with paths `src/{Parsers,Common,IO,Interpreters,Client}`, `base/base`, `programs/format`, `tests/queries/0_stateless`,
`utils/antlr`, `docs/reference`.
`docs/en/sql-reference` does not exist at this tag; the docs are Mintlify `.mdx` files under `docs/reference/`.
Paths below are relative to the clone.
**[T]** = tested with the binary, **[R]** = read in the source.
The scripts and data files mentioned at the end lived in the job's scratch directory and were not kept;
the recipes in `docs/MILESTONES.md` reproduce them.

## Verified / refuted

1. **Hand-written recursive descent with a separate lexer: verified [R].**
   - `src/Parsers/Lexer.{h,cpp}` produces the tokens.
     The parsers are `IParserBase::parseImpl` combinators (`ParserKeyword` and `ParserToken` in `CommonParsers.h`).
     Backtracking is limited by `max_parser_backtracks`/`max_depth` (`IParser.h`).
   - Expressions use an explicit operator-priority stack
     (`Operator`/`Layer` in `ExpressionListParsers.cpp`),
     which is a Pratt-style parser rather than pure recursive descent.
     No parser generator is involved.
2. **Comments never reach the AST: verified.
   "Skipped before parsing" is imprecise [R].**
   - Lexing is lazy.
     `Tokens::operator[]` (`TokenIterator.h:53`) drops `Whitespace`/`Comment` tokens as the parser pulls them.
     `skip_insignificant=true` is the default in `tryParseQuery` (`parseQuery.cpp:457`).
   - So comments may sit between the words of a compound keyword (`ORDER/*x*/BY`).
   - `COMMENT 'x'` clauses are string literals and do survive.
3. **`clickhouse-format` prints with the same code as `SHOW CREATE`: verified [R].**
   - `programs/format/Format.cpp:306` calls `res->format(buf, IAST::FormatSettings(oneline))`.
   - `SHOW CREATE` (`src/Interpreters/InterpreterShowCreateQuery.cpp:209`) calls `format({.one_line=false})`,
     which goes through `IAST::formatWithPossiblyHidingSensitiveData` to `IAST::format`
     and then the virtual `formatImpl`.
   - `FormatSettings` (`src/Parsers/IAST.h:360`) has: `one_line`, `identifier_quoting_rule/style`, `show_secrets`,
     `nl_or_ws`, `literal_escaping_style`, `print_pretty_type_names`, `enforce_strict_identifier_format`,
     `collapse_identical_nodes_to_aliases`, `ignore_redundant_parentheses`.
   - `hilite` and `max_line_length` are not FormatSettings fields.
     Highlighting is applied after formatting (`Format.cpp:309`, replxx).
   - `max_line_length` means "print on one line if the statement is shorter than N".
     It is not a wrap width.
     It must be under 256 and cannot be combined with `--oneline` [T].
4. **Debug builds check that the formatted AST parses back: verified, with details [R].**
   - Location: `src/Interpreters/executeQuery.cpp:2560-2700`, under `#ifndef NDEBUG`, in query execution.
     It is not in `clickhouse-format` or `parseQuery.cpp`.
   - Steps:
     1. Format the AST on one line (backtick quoting, `WhenNecessary`, `show_secrets`) and parse the text again.
        If that fails: `LOGICAL_ERROR` "Inconsistent AST formatting … cannot parse query back".
     2. Compare `getTreeHash(false)` of the two ASTs.
     3. Format the second AST and require the same string.
   - `NOT_IMPLEMENTED` is tolerated (MySQL ASTs).
   - Release binaries skip this check.
     Upstream CI runs debug builds on the stateless tests, so the round trip is tested there.
5. **`utils/antlr` still exists [R].**
   - It has `ClickHouseLexer.g4` (339 lines), `ClickHouseParser.g4`
     (581 lines), and a README saying "This parser is unsupported … not the parser of ClickHouse".
   - Last touched 2026-08-14 (`69f749b589` "Remove WINDOW VIEW", via the GitHub API).
     It only gets deletions.
   - It is stale: block comments don't nest, and it has no `#` or `//` comments, no heredocs and no query parameters.
     It has about 236 keyword rules against 422 single-word keywords in the enum.
6. **Other DESIGN.md claims [T]:**
   - `select key` and `SELECT key` both give `SELECT key`; `SELECT KEY` gives `SELECT KEY`.
   - `String` and `STRING` are both kept as written by the formatter.
     `STRING` fails at execution.
   - Rewrites confirmed: `$$…$$` → `'…'`, `a ? b : c` → `if(a, b, c)`, `INTERVAL 24 HOUR` → `toIntervalHour(24)`.
   - `{x:UInt8}` is kept, and `SETTINGS` stays on one line.
   - **Correction:** `--comments` keeps only `--` comments between statements.
     `/* */` and `#` comments are dropped even there (`skipSpacesAndComments`, `src/Client/ClientBaseHelpers.cpp:699`).
     A trailing `-- c` after `;` is moved to its own line.

**New facts that matter for the oracle:**

- **Secrets are hidden by default [T].**
  - `ReplicatedMergeTree('/p','{replica}')` comes out as `('[HIDDEN]','[HIDDEN]')`.
  - `CREATE USER … BY 'x'` loses `BY 'x'`.
  - So `--verify` must pass `--show_secrets`.
- **`clickhouse-format -n` cuts `INSERT … VALUES` data at the first newline** (`Format.cpp:289`).
  - Multi-line VALUES therefore fails.
    This happens in chum's `question_mark_migrations/0001_question_marks.up.sql`
    and in about 237 of the 0_stateless files.
  - clickhouse-client handles it properly (`src/Client/ClientBase.cpp:3461-3497`):
    - Values: it parses rows up to `;`.
    - Any other format: the data ends at the first `\n\n` or at EOF.
  - chairlift should copy the client's rule.
- `INSERT … FORMAT <not Values>` with inline data fails with `NOT_IMPLEMENTED`.
- In `INSERT … SELECT … SETTINGS s`, the output repeats the SETTINGS before the SELECT.
  The output is stable when formatted again.
- **The lexer can be built on its own**
  (`-DLEXER_STANDALONE_BUILD`, `src/Parsers/LexerStandalone.h`, C API in `clickhouse_lexer.h`,
  `src/Parsers/CMakeLists.txt:37`).
  It has no dependencies, so it could serve as a test-only reference to compare chairlift's Rust lexer against.

## Lexer checklist (`src/Parsers/Lexer.cpp`)

**TokenType list:**

- **Normal tokens:**
  `Whitespace Comment BareWord Number StringLiteral QuotedIdentifier OpeningRoundBracket ClosingRoundBracket OpeningSquareBracket ClosingSquareBracket OpeningCurlyBrace ClosingCurlyBrace Comma Semicolon VerticalDelimiter Dot Asterisk HereDoc DollarSign Plus Minus Slash Percent Arrow QuestionMark Colon Caret DoubleColon Equals NotEquals Less Greater LessOrEquals GreaterOrEquals Spaceship PipeMark PipeOperator Concatenation Tilde TildeAsterisk NotTilde NotTildeAsterisk At DoubleAt`
- **End:** `EndOfStream`
- **Error tokens (all after EndOfStream):**
  `Error ErrorMultilineCommentIsNotClosed ErrorSingleQuoteIsNotClosed ErrorDoubleQuoteIsNotClosed ErrorBackQuoteIsNotClosed ErrorSingleExclamationMark ErrorSinglePipeMark ErrorWrongNumber ErrorMaxQuerySizeExceeded`

| Construct | Rule (Lexer.cpp lines) | Tested |
| --- | --- | --- |
| Whitespace | ASCII ` \t\n\r\f\v`, plus Unicode U+0085, U+00A0, U+180E, U+2000–200D, U+2028/9, U+202F, U+205F, U+2060, U+3000 and BOM U+FEFF (`StringUtils.h:218`) | NBSP and BOM OK |
| `--` comment | Runs to `\n` (newline not included, cannot be escaped), `327-339`. `1--1` is `1` plus a comment | yes |
| `//` comment | Same as `--`, `344-353` | yes |
| `#` comment | Only `#` (space after) or `#!`. `#x` and `#<tab>` give `Error` (`383-394`) | yes |
| `/* */` | Nests, with a depth counter (`357-379`). Unclosed is an error | yes |
| `'…'` | Escapes: `''` and backslash plus any character. A backslash at EOF is an error (`24-57`) | yes |
| `x'..'`, `b'..'` | Hex and binary string literals; prefix in any case (`83-111`) | gives `'ABC'` |
| `‘…’`, `“…”` | Unicode-quoted string and identifier, no escapes (`480-496`) | yes |
| `` `…` ``, `"…"` | QuotedIdentifier; same escapes as strings (doubled quote or backslash) | yes |
| `e'…'` | Not supported | parse error |
| Heredoc | `$tag$…$tag$` with tag `[A-Za-z0-9_]*` (empty allowed); the first exact closer ends it. Without a closer, `$tag$abc` is a BareWord. A `$` not followed by a word character is `DollarSign` (`500-536`) | yes |
| BareWord | `[A-Za-z0-9_$]+`, ASCII only (`blåbær` gives Unrecognized token). May start with a digit: `1abc`, `0x`, `1e`, `1_` are BareWords (`243-256`) | yes |
| Number | Decimal, `0x`, `0b`, fraction, `e±` exponent (`p±` for hex). `_` between digits is allowed (`1_000`, `0xFF_FF`, in the exponent too; `isNumberSeparator`, `StringUtils.h:189`). Never signed: `-1` is Minus then Number | yes |
| Leading `.` | `.5` is a Number, unless the previous significant token is `)`, `]`, BareWord, QuotedIdentifier or Number; then `.` is a Dot. After a Dot, only `[0-9_]` is read, so `x.1.2` is tuple access. **This rule depends on the previous token** (`171-176`, `285-323`) | yes |
| inf, nan, true, false, null | BareWords; the parser decides what they are | yes |
| Multi-char operators | `->` `::` `<=>` `<=` `>=` `<>` `!=` `==` `\|\|` `\|>` `~*` `!~` `!~*` `@@` `\G`. A lone `!` is an error; a lone `\|` is PipeMark (`ErrorSinglePipeMark` is never produced). U+2212 is Minus | yes |
| Single-char | `( ) [ ] { } , ; . * + - / % ? : ^ ~ @ < > =`. There is no `..` token, and `[]` is two tokens | |
| `{name:Type}` | Not a token: `{`, BareWord, `:`, the type's tokens, `}`. The parser handles it (`ExpressionElementParsers.cpp:380`). Jinja `{{` is two `{` tokens | yes |
| INSERT data | The lexer knows nothing about it. `ParserInsertQuery.cpp:282-314` starts the data after the format name or `VALUES`, skipping `\t\f` and one `\r?\n`; the data runs to the end of input and the caller decides where it stops. `tryParseQuery` skips lexer-error checks for INSERT (`parseQuery.cpp:517,574`) | yes |
| Limits | Going past `max_query_size` gives an error token. Unknown bytes give `Error`, together with their UTF-8 continuation bytes | |

## Keywords

- **Definition [R].**
  `APPLY_FOR_PARSER_KEYWORDS` plus `…_WITH_UNDERSCORES` in `src/Parsers/CommonParsers.h` give **666** `Keyword` entries:
  - 422 single words and 244 compounds such as `ORDER BY` or `ADD COLUMN`;
  - **514 distinct words** in all.
- **Matching [R].**
  `ParserKeyword::parseImpl` (`CommonParsers.cpp:96`) compares a sequence of BareWord tokens case-insensitively,
  requiring the exact length.
- **Keywords outside the enum [R].**
  Extra keyword strings are built at runtime for SYSTEM commands, access-entity names
  and server types (`ParserSystemQuery.cpp:286,348,884`).
- **No reserved words for identifiers [R].**
  `ParserIdentifier` (`ExpressionElementParsers.cpp:344`) accepts any BareWord.
- **The only reserved list is for implicit aliases [R].**
  `ParserAlias::restricted_keywords` (`ExpressionElementParsers.cpp:2082`) has 44 words
  (FROM, WHERE, LIMIT, SETTINGS, FORMAT, JOIN, ON, …).
  It applies only to aliases written without `AS`: `SELECT 1 AS limit` works, `SELECT 1 limit` fails [T].
- **Words the formatter backquotes [R].**
  - `src/IO/WriteHelpers.cpp:95-150` backquotes
    `inf infinity nan true false distinct all some table select cube rollup recursive from top values`, plus `null` and
    anything not matching `[A-Za-z_][A-Za-z0-9_]*`.
  - Example [T]: `order, limit, settings, from, select` comes out as `order, limit, settings`, `` `from` ``,
    `` `select` ``.
- **What the oracle uppercases [T].**
  Only keywords that `formatImpl` prints as fixed strings: clause words,
  `AS NOT AND IS NULL ASC DESC OVER TTL CODEC DEFAULT …`.
  - Kept as written: identifiers, aliases, function names, type names
    (`STRING`, `nullable(string)`), engines (`mergetree`), setting names, codecs (`zstd`).
  - Lowercased: `TRUE`/`FALSE` and `NaN`/`INF`.
- **Case sensitivity at execution [T].**
  - Functions: `count`, `sum`, `if`, `concat`, `lower`, `length`, `now` work in any case.
    `toStartOfHour`, `toString`, `uniq` do not.
  - Types: `int/INT`, `DATETIME`, `DATETIME64`, `DATE/date`, `bool`, `DECIMAL`, `VARCHAR`, `TEXT`, `double` work.
    `string`, `STRING`, `nullable`, `INT32`, `UINT8`, `uuid`, `LOWCARDINALITY` fail.
  - Engines: `memory` fails.
    Settings: `MAX_THREADS` fails.

**Proposed re-casing rule.**
Re-case a word only when all three hold:

1. It is a BareWord: not quoted, and not inside a verbatim or Jinja region.
2. Its uppercase form is one of the 514 keyword words.
3. chairlift's parser consumed it in a **keyword slot**:
   - a statement or clause keyword;
   - an operator word: AND, OR, NOT, IN, LIKE, IS, BETWEEN, AS, ON, USING, CASE/WHEN/THEN/ELSE/END, INTERVAL, DISTINCT;
   - a column-definition keyword: DEFAULT, MATERIALIZED, ALIAS, CODEC, TTL, COMMENT;
   - `NULL`, or a `DATE`/`TIMESTAMP` literal prefix.

Never re-case function, type, engine, setting, layout or codec names, aliases, or `true`/`false`/`inf`/`nan`
(the plan later decided to re-case those literals to match clickhouse-format; see `docs/MILESTONES.md`).
Being in the keyword list is necessary but not sufficient: `key`, `date`, `name`, `type`,
`day` are all keywords and common column names.

## Corpus: 0_stateless

- **Size:** 12,619 `.sql` and 241 `.sql.j2` files.
- **How the split was done:** on `;` outside strings, quoted identifiers, heredocs and comments,
  with Jinja tags blanked.
- **How approximate it is:** a `;` inside inline INSERT data would be split wrongly.
  Estimated error about 1%; not measured.
- **Statement counts:** 268,052 in `.sql` files and 7,100 in `.j2` files.
- **Statements per file:** median 12, p95 70, max 1,931.

**`clickhouse-format -n` on 12,619 .sql files**
(10 parallel jobs, 20 s timeout): 12,048 parse (95.5%), 571 fail, 0 timeouts.

| Failure reason | Files |
| --- | --- |
| Multi-line INSERT VALUES cut at newline (heuristic: error at line 1, column 2 of a continuation row; sampled and confirmed) | 237 |
| INSERT with inline non-Values data (JSONEachRow 38, JSONAsObject 26, CSV 11, …) | 91 |
| Syntax: expected expression element | 48 |
| Syntax: expected Query (negative tests) | 47 |
| Empty query (`-- c` then `;`, or a stray `;`) | 32 |
| Syntax: expected token | 20 |
| Syntax: expected token sequence | 11 |
| Syntax: expected OR | 5 |
| Syntax: expected identifier | 4 |
| Unmatched parentheses | 3 |

**Timing (cost of an optional `--verify` step):**

| Run | Mean | p50 | p95 | Max |
| --- | --- | --- | --- | --- |
| Whole corpus, 10 parallel jobs (196 s wall, 1,659 s summed) | 131 ms | 125 | 198 | 7,969 ms |
| 300-file sample, sequential, `-n` | 89 ms | 84 | 127 | 195 |
| Same sample, `-n --oneline` | 85 ms | 80 | 111 | 247 |
| Startup only (`SELECT 1`) | 75 ms | 72 | 85 | 87 |

- Startup is about 72 ms of each run; parsing and printing take about 10 ms per file.
- The two 250 KB `multiIf`/`CASE` files take 1.2 s when run alone.
- Batching several files per process is the main saving.

**Statement kinds, 0_stateless (275,152 statements):**

| Kind | n | % | Kind | n |
| --- | --- | --- | --- | --- |
| SELECT | 141,953 | 51.6 | WITH (CTE select) | 2,924 |
| DROP TABLE | 33,777 | 12.3 | EXPLAIN (all) | 2,945 |
| INSERT | 25,240 | 9.2 | SYSTEM (all) | 3,886 |
| CREATE TABLE | 21,510 | 7.8 | OPTIMIZE | 1,597 |
| SET | 20,439 | 7.4 | CREATE / DROP DICTIONARY | 768 / 1,066 |
| ALTER TABLE | 4,818 | 1.8 | CREATE VIEW / MATERIALIZED VIEW | 627 / 541 |
| SHOW CREATE | 1,013 | 0.4 | CREATE DATABASE / USER / ROW POLICY | 378 / 184 / 290 |

**Local corpora** (chum 32 statements, tally 14, workshop 22; 68 in total):

| Kind | n |
| --- | --- |
| SELECT | 19 |
| CREATE TABLE | 12 |
| DROP TABLE | 8 |
| CREATE DATABASE | 5 |
| DROP DATABASE | 5 |
| INSERT | 3 |
| ALTER TABLE | 3 |
| GRANT, ALTER USER, CREATE MATERIALIZED VIEW, DROP VIEW | 2 each |
| CREATE ROLE, DROP ROLE, CREATE VIEW, CREATE DICTIONARY, DROP DICTIONARY | 1 each |

- Every plain `.sql` file in the local corpora passes the oracle except chum's multi-line VALUES file.
- All 6 Jinja files fail, as expected.

**Suggested order for teaching chairlift statements:**

1. SELECT / WITH
2. CREATE TABLE
3. DROP / SET / INSERT (a simple head with a verbatim tail)
4. ALTER TABLE
5. CREATE [MATERIALIZED] VIEW
6. DATABASE / DICTIONARY
7. GRANT, ROLE, USER
8. SYSTEM / EXPLAIN kept verbatim

## Oracle edge cases [T]

| Case | Result |
| --- | --- |
| `-n`, comments between and inside statements | All dropped: `SELECT 1;⏎⏎SELECT 2;⏎⏎SELECT 3;` |
| `-n --comments` | Keeps `-- head`, `-- after1`, `-- tail`. Drops `/* */`, `#` and comments inside statements |
| Input that is only a comment | rc 0, empty output |
| `-- c⏎;` | rc 62 `Empty query` |
| `INSERT … VALUES (1,'x'),(2,'y');` | Kept |
| Same, with rows on several lines | rc 62 (cut at the newline) |
| `FORMAT Values (…)` | Becomes `VALUES (…)` |
| `FORMAT CSV` / `FORMAT JSONEachRow` with data | rc 48 `Can't format INSERT query with data format 'CSV'` |
| `INSERT … SELECT … SETTINGS` | SETTINGS repeated before the SELECT |
| `{x:UInt8}`, `{tbl:Identifier}`, `{s: String}` | Kept; the space is removed (`{s:String}`) |
| `# c`, `#!shebang`, `// c` | Comments |
| `#nospace`, `#<tab>` | rc 62 `Unrecognized token` |
| `SELECT 1 -- trailing` with no final newline | `SELECT 1` (with `-n`: `SELECT 1;`) |
| `-n --oneline` | One line per statement, blank lines between |
| `--max_line_length 40` | Short statements on one line, long ones on several |
| `--max_line_length` together with `--oneline` | rc 2, "mutually exclusive" |
| `{{ x \| f }}` | rc 62 `Syntax error (query): failed at position N ({): … Expected substitution name (identifier).` |
| `{% if %}` | rc 62 `failed at position N (%) … Expected one of: substitution name (identifier), …` |
| `{# #}` | rc 62, failing at position 1 on `{` |
| `select key` / `SELECT key` / `SELECT KEY` | `SELECT key`, `SELECT key`, `SELECT KEY` |
| `String` / `STRING` in CREATE TABLE | Both kept as written; `STRING` fails at execution |
| `$$it's$$`, `$tag$a $$ b$tag$` | `'it\'s'`, `'a $$ b'` |
| Unclosed `$tag$abc` | The identifier `` `$tag$abc` `` |
| Trailing comma, where accepted | Accepted and dropped in a CREATE column list, a SELECT list (even `SELECT a, b, FROM t`), `IN (1,2,)` and a tuple |
| Trailing comma, where rejected | Function arguments `f(1,2,)` and ORDER BY |
| Numbers | `1_000_000` → `1000000`, `0x1.8p3` → `12.`, `1e3` → `1000.`, `NaN` → `nan` |
| `TRUE` | `true` |
| `\G` | rc 62 |
| ‘…’ and “…” | `'str' AS id` |

**How to tell the error kinds apart:**

- `substitution name`, or a failure at `{` or `%`: unrendered Jinja, so the oracle cannot judge the input.
- `Can't format INSERT query`, `Empty query`, or the **input** already fails to parse: also out of the oracle's reach.
- Only "the input parses but the output doesn't", or "the canonical forms differ", means chairlift broke the SQL.

## Docs examples

- **Size:** `docs/reference/**/*.mdx` has 1,379 files with **6,955** ```` ```sql ```` blocks in 787 files
  (4,552 under functions, 992 statements, 613 engines).
  The median block is 1 line long.
- **Pass rate:** 4,363 blocks (62.7%) pass `clickhouse-format -n`.
- **Failures:** about 2,000 are signature lines such as `f(x[, y])`, and 346 are multi-line VALUES.
- **Verdict:** usable as a corpus if filtered through the oracle first.

## Open questions

- Should `--verify` run one `clickhouse-format -n --oneline --show_secrets` process per batch instead of per file?
  It saves about 72 ms per file, but one bad statement stops the whole batch.
- How should the oracle step handle multi-line INSERT VALUES: strip the data, or join the rows onto one line?
- Should chairlift re-case `true`/`false`/`null`?
  The oracle can't catch it either way: it lowercases `true`/`false` and uppercases `NULL`.
- The alias-reserved list and the formatter's quote list differ
  (`order` and `limit` are reserved as implicit aliases but not quoted).
  Does chairlift need either, given it never changes quoting?
