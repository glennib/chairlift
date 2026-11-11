//! Every ClickHouse keyword word and literal word, in lower, capitalised and
//! upper case, in every position of [`POSITIONS`] and [`STATEMENTS`]: the
//! parser's keyword slots must pass the verifier, and `clickhouse-format`
//! must print the re-cased text as it prints the input. A word re-cased where
//! ClickHouse reads a name (`columns(1)`, `* EXCEPT null`) changes what the
//! oracle prints, since names keep their case. [`KEYWORDS_IN_LOWER_CASE`]
//! adds statements whose own keywords the oracle judges.
//!
//! The oracle is `CHAIRLIFT_CLICKHOUSE_FORMAT`, by default `clickhouse-format`
//! on `PATH`; without it the test skips, unless `CHAIRLIFT_REQUIRE_CORPUS` is
//! set. `mise run test:keyword-positions` runs it alone.
//!
//! To cover a new position, add a template. In [`POSITIONS`] it is an
//! expression in which `{w}` stands for the word, parsed as an expression and
//! judged as `SELECT 0, <expression>`, so that a word that begins it is not
//! read as `SELECT`'s `DISTINCT` or `ALL`. In [`STATEMENTS`] it is a whole
//! statement, parsed and judged as one, in which `{w}` stands for the word
//! and `{n}` for the word where only a name can stand, which must then never
//! be a keyword slot, whatever the oracle says.

use std::ops::Range;

use chairlift::keyword::Keyword;
use chairlift::lexer::Mode;
use chairlift::oracle::Judgement;
use chairlift::oracle::Oracle;
use chairlift::oracle::Verdict;
use chairlift::oracle::judge;
use chairlift::syntax::Entry;
use chairlift::syntax::SyntaxKind;
use chairlift::syntax::parse_as;
use chairlift::verify::verify;

/// Expressions with `{w}` where the word goes.
#[rustfmt::skip]
const POSITIONS: &[&str] = &[
    // Names and calls.
    "{w}", "t.{w}", "db.t.{w}", "{w}.x", "{w}.{w}", "{w}(x)", "{w}(x, y)", "{w}()",
    "{w}(x)(y)", "{w}(distinct x)", "{w}(*)", "{w}(1)", "{w}(f(x))", "{w}('a', 'b')",
    "{w}((select 1))", "f({w})", "f(x, {w})", "f({w}, {w})", "f({w} as y)", "f(x as {w})",
    "count(distinct {w})", "array({w})", "map({w}, 1)", "tuple({w})", "if({w}, 1, 2)",
    "f(x).{w}", "tuple(1).{w}", "{w}.1", "x.{w}.1", "{w}[1]", "{w}[{w}]", "@@{w}",
    // Aliases.
    "x as {w}", "x {w}", "{w} {w}",
    // Lambdas.
    "arrayMap({w} -> {w}, a)", "arrayMap(({w}, y) -> y, a, b)",
    // Operators.
    "{w} + 1", "1 + {w}", "-{w}", "not {w}", "{w} = 1", "{w} in (1)", "x in {w}",
    "{w} between 1 and 2", "x between {w} and {w}", "{w} is null", "x is {w}", "x is not {w}",
    "{w} like 'a'", "x like {w}", "{w} || 'a'", "{w} * 2", "{w} mod 2", "x {w} 2",
    "x not {w} y", "{w} is not distinct from {w}", "x ? {w} : {w}", "{w} ? 1 : 2",
    "{w} at time zone 'UTC'", "x at {w}",
    // Quantifiers: only one expression or a subquery makes one.
    "x = {w}(1)", "x = {w}(1, 2)", "x = {w}()", "x = {w}(y as z)", "x < {w}(y, z) + 1",
    "x = {w}((select 1))", "x = {w}((select 1), 2)", "x = {w} (select 1)", "x like {w}(y)",
    "x like {w}(y, z)", "x is distinct from {w}(y)", "x ~ {w}(y)",
    // Collections, and brackets that hold a query only where ClickHouse reads
    // one.
    "[{w}]", "({w}, 1)", "({w})", "{{'a': {w}}}", "({w} - 1)", "({w} [1])", "({w} in (1))",
    "({w} and x)", "({w} is null)", "x in ({w} - 1)", "({w} (1))", "({w} (x))", "({w} (x, y))",
    // CASE, INTERVAL, parameters.
    "case {w} when {w} then {w} else {w} end", "case when {w} then 1 end",
    "interval {w} day", "interval 1 {w}", "{{{w}:String}}", "{{x:{w}}}",
    // Column matchers and transformers.
    "{w}.*", "t.{w}.*", "columns({w})", "columns(a, {w})", "columns('a') apply {w}",
    "columns('a') except ({w})", "* except ({w})", "* except {w}", "* except ({w}, a)",
    "* except strict {w}", "* replace (1 as {w})", "* replace ({w} as x)", "* apply {w}",
    "* apply({w})", "* apply({w}) apply {w}", "* apply {w}(0.5)", "* apply(x -> {w})",
    "* replace ({w} + 1 as x)", "columns(a.{w})", "columns({w}) apply {w}",
    // Windows.
    "f(x) over {w}", "f(x) over ({w})",
    // Functions with syntax of their own.
    "cast(x as {w})", "cast(x, '{w}')", "dateAdd({w}, 1, d)", "dateDiff({w}, a, b)",
    "extract({w} from d)", "substring({w} from 1)", "position({w} in s)", "trim({w})",
    "trim(both {w} from s)",
    // Data types.
    "x::{w}", "{w}::String", "x::Nullable({w})", "x::Map(String, {w})", "x::{w}(10)",
    "x::{w}(3, 'UTC')", "x::Enum8('{w}' = 1)", "cast(x as Tuple({w} String))",
    "x::Tuple({w} String, b {w})", "x::AggregateFunction({w}, String)",
    "x::AggregateFunction(1, {w}, String)", "x::SimpleAggregateFunction({w}, String)",
    "x::AggregateFunction({w}(0.5), String)",
    "x::JSON({w} String)",
];

/// Statements with `{w}`, or `{n}` in a name position, where the word goes.
const STATEMENTS: &[&str] = &[
    // SELECT, upper case but for the word.
    "SELECT {w} FROM t",
    "SELECT a, {w} FROM t",
    "SELECT a AS {w} FROM t",
    "SELECT a {w} FROM t",
    "SELECT DISTINCT {w} FROM t",
    "SELECT ({w}) FROM t",
    "SELECT ({w}, 1) FROM t",
    "SELECT {w}(1) FROM t",
    "SELECT * FROM {w}",
    "SELECT * FROM {w}.t",
    "SELECT * FROM db.{w}",
    "SELECT * FROM {w}(1)",
    "SELECT * FROM t AS {w}",
    "SELECT * FROM t {w}",
    "SELECT * FROM t {w} WHERE 1",
    "SELECT * FROM t AS x ({w})",
    "WITH {w} AS (SELECT 1) SELECT * FROM {w}",
    "WITH 1 AS {w} SELECT {w}",
    "SELECT * FROM a JOIN b USING ({w})",
    "SELECT * FROM a JOIN b USING {w}",
    "SELECT * FROM a JOIN b ON {w} = 1",
    "SELECT * FROM a JOIN {w} ON 1",
    "SELECT * FROM t ARRAY JOIN {w}",
    "SELECT * FROM t PREWHERE {w}",
    "SELECT * FROM t WHERE {w}",
    "SELECT * FROM t WHERE {w} = 1",
    "SELECT * FROM t GROUP BY {w}",
    "SELECT * FROM t GROUP BY a, {w}",
    "SELECT * FROM t HAVING {w}",
    "SELECT * FROM t QUALIFY {w}",
    "SELECT * FROM t WINDOW {w} AS ()",
    "SELECT * FROM t ORDER BY {w}",
    "SELECT * FROM t ORDER BY {w} DESC",
    "SELECT * FROM t ORDER BY a, {w}",
    "SELECT x FROM t ORDER BY x WITH FILL INTERPOLATE ({w})",
    "SELECT * FROM t LIMIT {w}",
    "SELECT * FROM t LIMIT 1 BY {w}",
    "SELECT * FROM t LIMIT 1 OFFSET {w}",
    "SELECT * FROM t SETTINGS {w} = 1",
    "SELECT * FROM t SETTINGS {n}",
    "SELECT * FROM t SETTINGS a = 1, {n}",
    "SELECT * FROM t FORMAT {w}",
    "SELECT * FROM t LIMIT {w} BY a",
    "SELECT * FROM t OFFSET {w}",
    "SELECT * FROM t AS {w} FINAL",
    "SELECT * FROM t1 {w} JOIN t2 ON 1",
    "SELECT * FROM (SELECT 1) {w}",
    "SELECT * FROM numbers(1) {w}",
    "WITH {w} SELECT 1",
    "FROM {w} SELECT 1",
    "FROM t {w} SELECT 1",
    // An alias without `AS`, before what may follow a table.
    "SELECT * FROM t {w} FINAL",
    "SELECT * FROM t {w} SAMPLE 1",
    "SELECT * FROM t {w} ARRAY JOIN a",
    "SELECT * FROM t {w} FORMAT JSON",
    "SELECT * FROM t {w} CROSS JOIN u",
    "SELECT * FROM t {w} (a)",
    "SELECT * FROM t {w} GROUP BY 1",
    "SELECT 1 FROM t {w} EXCEPT SELECT 1",
    "FROM t {w} FINAL",
    "FROM t {w}, u",
    "FROM a JOIN b {w} ON 1",
    "SELECT * FROM t FINAL ({n})",
    "SELECT * FROM (a JOIN b ON 1) {w}",
    "SELECT * FROM viewIfPermitted(SELECT 1 ELSE {n}(1))",
    "SELECT * FROM viewIfPermitted(SELECT 1 FROM t ELSE {n}('a UInt8'))",
    "SELECT * FROM viewIfPermitted(SELECT a FROM t WHERE {w} ELSE null('a UInt8'))",
    "SELECT {w} FROM t UNION ALL SELECT 1",
    "SELECT * FROM (SELECT {w} FROM t)",
    "SELECT * FROM t WHERE a IN (SELECT {w} FROM u)",
    "CREATE TABLE t ENGINE = Memory AS SELECT a FROM u {w}",
    "CREATE MATERIALIZED VIEW v TO t AS SELECT a FROM u {w}",
    "CREATE TABLE t AS {n}.{n}",
    "CREATE TABLE t AS {w}(1)",
    "CREATE TABLE t AS {w}()",
    "CREATE TABLE t (a UInt8) AS {n}('x')",
    "CREATE TABLE t AS {w} ENGINE = Memory",
    "CREATE TABLE t (a UInt8) ENGINE = Kafka SETTINGS {n} = 'x'",
    "CREATE TABLE t (a UInt8) ENGINE = Kafka SETTINGS a = 1, {n} = 'x'",
    "CREATE TABLE t (a UInt8) ENGINE = Memory SETTINGS {n}, a = 1",
    "CREATE TABLE t (a UInt8, d Date) ENGINE = MergeTree ORDER BY a TTL d GROUP BY {w} SET {n} = max({w})",
    "CREATE TABLE t (a UInt8, d Date) ENGINE = MergeTree ORDER BY a TTL d GROUP BY a, {w}",
    "CREATE TABLE t (a UInt8, d Date) ENGINE = MergeTree ORDER BY a TTL d, {w}",
    "CREATE TABLE t (a UInt8, d Date) ENGINE = MergeTree ORDER BY a TTL {w}",
    "CREATE MATERIALIZED VIEW v REFRESH EVERY 1 HOUR SETTINGS {n} = 1 TO t AS SELECT 1",
    "CREATE MATERIALIZED VIEW v TO {n} AS SELECT 1",
    "CREATE TABLE t (a Int8, PROJECTION p (SELECT {w})) ENGINE = MergeTree ORDER BY a",
    "CREATE TABLE t (a Int8, PROJECTION p (SELECT a GROUP BY {w})) ENGINE = MergeTree ORDER BY a",
    "CREATE TABLE t (a Int8, PROJECTION p (SELECT a ORDER BY {w})) ENGINE = MergeTree ORDER BY a",
    // CREATE, lower case so that its keyword slots change when re-cased;
    // every word stands where a name goes.
    "create table {n}.{n} on cluster {n} (a UInt8) engine = {n} settings {n} = 1 format {n}",
    "create table t (a UInt8, index {n} a type {n} granularity 1, projection {n} (SELECT a), constraint {n} check a > 0) engine = MergeTree order by a",
    "create table t (a UInt8 codec({n}) statistics({n}), b UInt8) engine = MergeTree order by a ttl a group by a set {n} = max(b)",
    "create table t ({n} UInt8) engine = Memory",
    "create table t (a {n}) engine = Memory",
    "create materialized view {n}.{n} refresh every 1 hour depends on {n}.{n} to {n}.{n} as SELECT 1",
    // From the review of the `SELECT` and `CREATE` grammars, those of its
    // templates that reach a keyword slot or a name position no template
    // above does, with names where only a name stands.
    "SELECT * FROM a JOIN b ON {w} WHERE 1",
    "SELECT * FROM a JOIN b ON x = {w} AND {w}",
    "SELECT * FROM remote('h', {w}, {w})",
    "WITH x AS MATERIALIZED (SELECT 1) SELECT {w}",
    "WITH RECURSIVE {w} AS (SELECT 1) SELECT 1",
    "WITH {w} AS {n} SELECT 1",
    "SELECT 1 SETTINGS {n} = {w}",
    "SELECT count() OVER {n} FROM t WINDOW {n} AS ()",
    "SELECT a FROM t ORDER BY a WITH FILL INTERPOLATE ({n} AS {w})",
    "SELECT a FROM t ORDER BY a WITH FILL INTERPOLATE ({n} AS {w} + 1)",
    "SELECT * FROM t SAMPLE 1 / 10 OFFSET 1 WHERE {w}",
    "SELECT * FROM t LIMIT 1 OFFSET 1 BY {w}",
    "SELECT * FROM t LIMIT {w} WITH TIES",
    "SELECT * FROM t OFFSET {w} ROWS",
    "SELECT * FROM t OFFSET 1 ROWS FETCH FIRST {w} ROWS ONLY",
    "SELECT * FROM t ORDER BY a WITH FILL STALENESS {w}",
    "SELECT * FROM t ORDER BY a WITH FILL FROM {w} TO {w} STEP {w}",
    "SELECT * FROM t ORDER BY {w} COLLATE 'en'",
    "SELECT * FROM t ORDER BY {w} NULLS FIRST",
    "SELECT 1 FROM t GROUP BY {w}(a, b)",
    "SELECT 1 FROM t GROUP BY {w} WITH TOTALS",
    "SELECT 1 FROM t GROUP BY {w} WITH ROLLUP",
    "SELECT 1 FROM t GROUP BY GROUPING SETS ({w}, ())",
    "SELECT 1 FROM t GROUP BY {w} AS {n}",
    "SELECT TOP 1 WITH TIES {w} FROM t",
    "SELECT DISTINCT ON ({w}, a) {w} FROM t",
    "SELECT {w} {w} FROM t",
    "SELECT {w} AS {n}, 1 FROM t",
    "SELECT {w} + 1 {w} FROM t",
    "FROM {n} SELECT 1",
    "SELECT * FROM t WHERE {w} AND {w}",
    "SELECT * FROM t {w} ({n})",
    "SELECT * FROM t ORDER BY 1 {w}",
    "SELECT 1 INTO OUTFILE 'x' {w}",
    "SELECT * FROM t STREAM {w}",
    "SELECT 1 FROM t LIMIT 1 {w} 1",
    "SELECT 1 FROM t AS a {w} JOIN t AS b ON 1",
    "CREATE TABLE t (a UInt8) ENGINE = Distributed(c, db, t, {w}())",
    "CREATE TABLE t (a UInt8, INDEX i {w} TYPE minmax) ENGINE = MergeTree ORDER BY a",
    "CREATE TABLE t (a UInt8, INDEX i {w}({w}) TYPE minmax) ENGINE = MergeTree ORDER BY a",
    "CREATE TABLE t (a {w}(1)) ENGINE = Memory",
    "CREATE TABLE t (a Tuple({n} UInt8)) ENGINE = Memory",
    "CREATE TABLE t (a LowCardinality({w})) ENGINE = Memory",
    "CREATE TABLE t (a UInt8 DEFAULT {w} CODEC(ZSTD)) ENGINE = Memory",
    "CREATE TABLE t (a UInt8 DEFAULT {w} TTL d) ENGINE = Memory",
    "CREATE TABLE t (a UInt8 TTL {w}) ENGINE = Memory",
    "CREATE TABLE t (a UInt8 TTL {w} + INTERVAL 1 DAY) ENGINE = Memory",
    "CREATE TABLE t (a UInt8 NOT NULL DEFAULT {w}) ENGINE = Memory",
    "CREATE TABLE t (a String COLLATE {n}) ENGINE = Memory",
    "CREATE TABLE t (a UInt8 SETTINGS (x = {w})) ENGINE = Memory",
    "CREATE TABLE t (a UInt8 PRIMARY KEY, {n} UInt8) ENGINE = MergeTree",
    "CREATE TABLE t (a UInt8 COMMENT 'y') ENGINE = Memory COMMENT 'x'",
    "CREATE TABLE t (a UInt8, d Date) ENGINE = MergeTree ORDER BY a TTL d TO VOLUME 'x', {w} + INTERVAL 1 DAY",
    "CREATE TABLE t (a UInt8, d Date) ENGINE = MergeTree ORDER BY a TTL d RECOMPRESS CODEC({n})",
    "CREATE TABLE t (a UInt8, d Date) ENGINE = MergeTree ORDER BY a TTL d DELETE WHERE {w}",
    "CREATE TABLE t (a UInt8, d Date) ENGINE = MergeTree ORDER BY a TTL d {w}",
    "CREATE TABLE t (a UInt8) ENGINE = MergeTree PARTITION BY {w} ORDER BY a",
    "CREATE TABLE t (a UInt8) ENGINE = MergeTree ORDER BY {w} DESC",
    "CREATE TABLE t (a UInt8) ENGINE = MergeTree ORDER BY a SAMPLE BY {w} TTL d",
    "CREATE TABLE t (a UInt8) ENGINE = MergeTree ORDER BY {w} PRIMARY KEY {w}",
    "CREATE TABLE t (a UInt8, PROJECTION p INDEX {w} TYPE {n}) ENGINE = MergeTree ORDER BY a",
    "CREATE TABLE t (a UInt8, PROJECTION p (SELECT a) WITH SETTINGS ({n} = 1)) ENGINE = MergeTree ORDER BY a",
    "CREATE TABLE t (a UInt8, CONSTRAINT {n} CHECK {w}) ENGINE = Memory",
    "CREATE TABLE t (a UInt8, CONSTRAINT c ASSUME {w} = 1) ENGINE = Memory",
    "CREATE MATERIALIZED VIEW v (a UInt8) ENGINE = Memory AS SELECT {w}",
    "CREATE MATERIALIZED VIEW v REFRESH EVERY 1 YEAR OFFSET 1 {w} AS SELECT 1",
    "CREATE MATERIALIZED VIEW v REFRESH AFTER 1 YEAR RANDOMIZE FOR 1 {w} AS SELECT 1",
    "CREATE MATERIALIZED VIEW v REFRESH EVERY 1 HOUR DEPENDS ON {n}, {n}.{n} AS SELECT 1",
    "CREATE MATERIALIZED VIEW v REFRESH EVERY 1 HOUR APPEND TO {n} AS SELECT 1",
    "CREATE MATERIALIZED VIEW v TO t DEFINER = {w} AS SELECT 1",
    "CREATE MATERIALIZED VIEW v TO t SQL SECURITY {w} AS SELECT 1",
    "CREATE MATERIALIZED VIEW v TO t DEFINER = {n}@{n} AS SELECT 1",
    "CREATE TABLE t ENGINE = TimeSeries DATA {n}.{n} TAGS {n}",
    "CREATE TABLE t ENGINE = TimeSeries TAGS INNER UUID '01234567-89ab-cdef-0123-456789abcdef'",
    "CREATE TABLE t ENGINE = TimeSeries METRICS ENGINE = {n}() ORDER BY {w}",
    "CREATE TEMPORARY TABLE {n} (a UInt8) ENGINE = Memory",
    "CREATE OR REPLACE TABLE {n} (a UInt8) ENGINE = Memory",
    "CREATE TABLE IF NOT EXISTS {n}.{n} (a UInt8) ENGINE = Memory",
    "CREATE TABLE t UUID '01234567-89ab-cdef-0123-456789abcdef' ({n} UInt8) ENGINE = Memory",
    "CREATE TABLE t (a UInt8, PRIMARY KEY {w}) ENGINE = MergeTree",
    "CREATE TABLE t (a UInt8) ENGINE = MergeTree UNIQUE KEY {w}",
    "CREATE TABLE t ({n} UInt64 AUTO_INCREMENT) ENGINE = Memory",
    "CREATE TABLE t ({n} Nullable(Int8) NULL, b Int8 NOT NULL) ENGINE = Memory",
    "CREATE TABLE t (a Int8 SETTINGS ({n} = 1)) ENGINE = MergeTree ORDER BY a",
    "CREATE TABLE t (a Int8, CONSTRAINT {n} ASSUME a > 0) ENGINE = Memory",
    "SELECT count() {w} FROM t",
    "SELECT overlay({w} PLACING 'x' FROM 1)",
    "SELECT * FROM t LIMIT AFTER {w} ALL UNTIL {w}",
    "SELECT * FROM t ORDER BY {w} ASCENDING",
    "CREATE TABLE t (a Int8, PROJECTION {n} INDEX a TYPE basic WITH SETTINGS (index_granularity = 1)) ENGINE = MergeTree ORDER BY a",
    "CREATE MATERIALIZED VIEW v TO t DEFINER = {w} SQL SECURITY DEFINER AS SELECT 1",
    "SELECT * FROM {n} SAMPLE 1 / 10 OFFSET 1 / 2",
    "SELECT ALL {w} FROM t",
    "SELECT 1 SETTINGS {n} = DEFAULT",
    "SELECT * FROM t STREAM WATERMARK FOR {n} AS ts",
    "CREATE TABLE t (d Date) ENGINE = MergeTree ORDER BY d TTL d DELETE WHERE {w}",
    "CREATE TABLE t ({n} Int8) ENGINE = MergeTree ORDER BY {w} UNIQUE KEY {w}",
    "SELECT * FROM (VALUES ({w}))",
    "CREATE TABLE ts ENGINE = TimeSeries DATA {n}.{n} TAGS {n}",
    // INSERT: the target, the columns, the settings, the format and the
    // query; the data is never a keyword slot.
    "INSERT INTO {n} VALUES (1)",
    "INSERT INTO {n}.{n} (a) VALUES ({w})",
    "INSERT INTO TABLE {n} SELECT 1",
    "INSERT INTO t ({n}) VALUES (1)",
    "INSERT INTO t (a, {n}.{n}) VALUES (1)",
    "INSERT INTO t ({n}.*) VALUES (1)",
    "INSERT INTO t (* EXCEPT ({n})) VALUES (1)",
    "INSERT INTO t (COLUMNS({n})) VALUES (1)",
    "INSERT INTO t SETTINGS {n} = 1 VALUES (1)",
    "INSERT INTO t SETTINGS a = {w} SELECT 1",
    "INSERT INTO t FORMAT {n}",
    "INSERT INTO t FROM INFILE 'x' FORMAT {n}",
    "INSERT INTO FUNCTION {n}('x') SELECT 1",
    "INSERT INTO FUNCTION file('x') PARTITION BY {w} SELECT 1",
    "INSERT INTO t SELECT {w} FROM u",
    "INSERT INTO t ({n}) SELECT {w}",
    "INSERT INTO t SELECT * FROM u {w}",
    "INSERT INTO t SELECT * FROM u FORMAT {n}",
    "INSERT INTO t ({n}) {w}",
    "WITH {w} AS (SELECT 1) INSERT INTO t SELECT * FROM {w}",
    "WITH 1 AS {n} INSERT INTO t SELECT {w}",
    "WITH {w} INSERT INTO t SELECT 1",
    // Statements of a line or two.
    "DROP TABLE {n}",
    "DROP TABLE {n}.{n}, {n} ON CLUSTER {n}",
    "DROP TABLE IF EXISTS {n} SYNC",
    "DROP TABLE t {w}",
    "DROP TABLE {n} FORMAT {n}",
    "DROP TABLE t SETTINGS {n} = {w}",
    "DROP DATABASE {n}",
    "DROP DICTIONARY {n}.{n}",
    "DROP VIEW {n}",
    "DROP TEMPORARY TABLE {n}",
    "DROP FUNCTION {n}",
    "DROP NAMED COLLECTION {n}",
    "DROP INDEX {n} ON {n}.{n}",
    "DROP {w} t",
    "DETACH TABLE {n} PERMANENTLY",
    "DETACH DATABASE {n}",
    "TRUNCATE {n}",
    "TRUNCATE TABLE {n}.{n}",
    "TRUNCATE ALL TABLES FROM {n} LIKE 'x'",
    "UNDROP TABLE {n}.{n}",
    "ATTACH TABLE {n}.{n}",
    "ATTACH DICTIONARY {n}",
    "ATTACH DATABASE {n}",
    "ATTACH TABLE {n} (a UInt8) ENGINE = Memory",
    "ATTACH TABLE t {w}",
    "RENAME TABLE {n} TO {n}",
    "RENAME TABLE {n}.{n} TO {n}.{n}, a TO {n}",
    "RENAME a.{n} TO {n}",
    "RENAME DATABASE {n} TO {n}",
    "RENAME DICTIONARY {n} TO {n}",
    "EXCHANGE TABLES {n} AND {n}",
    "EXCHANGE DICTIONARIES {n}.{n} AND {n}",
    "OPTIMIZE TABLE {n}",
    "OPTIMIZE TABLE {n}.{n} FINAL",
    "OPTIMIZE TABLE t {w}",
    "OPTIMIZE TABLE t PARTITION {w}",
    "OPTIMIZE TABLE t PARTITION {w} FINAL",
    "OPTIMIZE TABLE t PARTITION ({w}, 1) FINAL",
    "OPTIMIZE TABLE t FINAL DEDUPLICATE BY {n}",
    "OPTIMIZE TABLE t DEDUPLICATE BY {n}, {n}",
    "OPTIMIZE TABLE t DEDUPLICATE BY * EXCEPT ({n})",
    "SET {n} = 1",
    "SET {n} = {w}",
    "SET a = 1, {n} = 'x'",
    "SET {n}",
    "SET {n}, a = 1",
    "SET {w} = DEFAULT",
    "SET TIME ZONE {w}",
    "USE {n}",
    "USE DATABASE {n}",
    // ALTER: the table, the names after each command, the definitions, the
    // lists and the conditions.
    "ALTER TABLE {n} DROP COLUMN {n}",
    "ALTER TABLE {n}.{n} ON CLUSTER {n} DROP COLUMN a",
    "ALTER TABLE t DROP COLUMN IF EXISTS {n}.{n}",
    "ALTER TABLE t ADD COLUMN {n} Int8",
    "ALTER TABLE t ADD COLUMN {n} Int8 FIRST",
    "ALTER TABLE t ADD COLUMN IF NOT EXISTS a Int8 AFTER {n}",
    "ALTER TABLE t ADD COLUMN a Int8 DEFAULT {w}",
    "ALTER TABLE t ADD COLUMN a Int8 {w}",
    "ALTER TABLE t ADD COLUMN a {n}",
    "ALTER TABLE t MODIFY COLUMN {n} Int8",
    "ALTER TABLE t MODIFY COLUMN {n} COMMENT 'x'",
    "ALTER TABLE t MODIFY COLUMN a {w}",
    "ALTER TABLE t MODIFY COLUMN a Int8 {w}",
    "ALTER TABLE t MODIFY COLUMN {n} REMOVE TTL",
    "ALTER TABLE t MODIFY COLUMN a REMOVE {w}",
    "ALTER TABLE t MODIFY COLUMN a MODIFY SETTING {n} = 1",
    "ALTER TABLE t MODIFY COLUMN a RESET SETTING {n}, {n}",
    "ALTER TABLE t ALTER COLUMN {n} TYPE Int8",
    "ALTER TABLE t RENAME COLUMN {n} TO {n}",
    "ALTER TABLE t RENAME COLUMN IF EXISTS {n}.{n} TO {n}.{n}",
    "ALTER TABLE t CLEAR COLUMN {n} IN PARTITION {w}",
    "ALTER TABLE t COMMENT COLUMN {n} 'x'",
    "ALTER TABLE t MATERIALIZE COLUMN {n}",
    "ALTER TABLE t MODIFY ORDER BY {w}",
    "ALTER TABLE t MODIFY SAMPLE BY {w}",
    "ALTER TABLE t MODIFY TTL d + INTERVAL 1 DAY {w}",
    "ALTER TABLE t MODIFY TTL {w}",
    "ALTER TABLE t MODIFY SETTING {n} = 1",
    "ALTER TABLE t MODIFY SETTING a = {w}",
    "ALTER TABLE t RESET SETTING {n}, {n}",
    "ALTER TABLE t ADD INDEX {n} {w} TYPE minmax",
    "ALTER TABLE t ADD INDEX {n} a TYPE {n} GRANULARITY 1",
    "ALTER TABLE t ADD INDEX i a TYPE minmax AFTER {n}",
    "ALTER TABLE t DROP INDEX {n}",
    "ALTER TABLE t MATERIALIZE INDEX {n} IN PARTITION ID 'x'",
    "ALTER TABLE t CLEAR INDEX IF EXISTS {n}",
    "ALTER TABLE t ADD PROJECTION {n} (SELECT {w})",
    "ALTER TABLE t DROP PROJECTION {n}",
    "ALTER TABLE t MATERIALIZE PROJECTION {n}",
    "ALTER TABLE t ADD CONSTRAINT {n} CHECK {w}",
    "ALTER TABLE t DROP CONSTRAINT {n}",
    "ALTER TABLE t ADD STATISTICS {n} TYPE {n}",
    "ALTER TABLE t DROP STATISTICS {n}, {n}",
    "ALTER TABLE t DELETE WHERE {w}",
    "ALTER TABLE t DELETE IN PARTITION {w} WHERE 1",
    "ALTER TABLE t UPDATE {n} = {w} WHERE 1",
    "ALTER TABLE t UPDATE a = 1, {n} = 2 WHERE {w}",
    "ALTER TABLE t DROP PARTITION {w}",
    "ALTER TABLE t DETACH PARTITION {w}",
    "ALTER TABLE t ATTACH PARTITION 1 FROM {n}.{n}",
    "ALTER TABLE t REPLACE PARTITION 1 FROM {n}",
    "ALTER TABLE t MOVE PARTITION 1 TO TABLE {n}",
    "ALTER TABLE t FREEZE PARTITION {w}",
    "ALTER TABLE t {w}",
    "ALTER TABLE t DROP COLUMN a, {w}",
    "ALTER TABLE t DROP COLUMN a SETTINGS {n} = 1",
    "ALTER TABLE mv MODIFY QUERY SELECT {w}",
    "ALTER DATABASE {n} MODIFY SETTING {n} = 1",
    "ALTER DATABASE {n} ON CLUSTER {n} MODIFY COMMENT 'x'",
    "DELETE FROM {n} WHERE {w}",
    "DELETE FROM {n}.{n} ON CLUSTER {n} IN PARTITION {w} WHERE 1",
    "DELETE FROM t WHERE 1 SETTINGS {n} = 1",
    "UPDATE {n} SET {n} = {w} WHERE 1",
    "UPDATE {n}.{n} SET a = 1 WHERE {w}",
    "UPDATE t SET a = 1 IN PARTITION {w} WHERE 1",
    // Views, databases, functions and dictionaries.
    "CREATE VIEW {n} AS SELECT {w}",
    "CREATE VIEW {n}.{n} ({n} UInt8) AS SELECT 1",
    "CREATE OR REPLACE VIEW {n} AS SELECT * FROM {n}",
    "CREATE VIEW v AS SELECT * FROM t {w}",
    "CREATE DATABASE {n}",
    "CREATE DATABASE {n} ENGINE = {n}",
    "CREATE DATABASE IF NOT EXISTS {n} ON CLUSTER {n} ENGINE = Atomic COMMENT 'x'",
    "CREATE FUNCTION {n} AS ({n}) -> {n} + 1",
    "CREATE FUNCTION {n} AS {n} -> {w}",
    "CREATE FUNCTION f ON CLUSTER {n} AS (x, {n}) -> x",
    "CREATE DICTIONARY {n} ({n} UInt64) PRIMARY KEY {n} SOURCE(CLICKHOUSE(TABLE 'x')) LAYOUT(FLAT()) LIFETIME(0)",
    "CREATE DICTIONARY {n}.{n} (a UInt64, {n} String) PRIMARY KEY ({n}) SOURCE(CLICKHOUSE(TABLE 'x')) LAYOUT(FLAT()) LIFETIME(0)",
    "CREATE DICTIONARY d (a UInt64 DEFAULT {w}) PRIMARY KEY a SOURCE(CLICKHOUSE(TABLE 'x')) LAYOUT(FLAT()) LIFETIME(0)",
    "CREATE DICTIONARY d (a UInt64 EXPRESSION {w}) PRIMARY KEY a SOURCE(CLICKHOUSE(TABLE 'x')) LAYOUT(FLAT()) LIFETIME(0)",
    "CREATE DICTIONARY d (a UInt64 {w}) PRIMARY KEY a SOURCE(CLICKHOUSE(TABLE 'x')) LAYOUT(FLAT()) LIFETIME(0)",
    "CREATE DICTIONARY d (a UInt64) PRIMARY KEY a SOURCE({n}({n} 'x')) LAYOUT({n}()) LIFETIME({n} 0 {n} 1)",
    "CREATE DICTIONARY d (a UInt64) PRIMARY KEY a SOURCE(CLICKHOUSE({n} {w})) LAYOUT({n}) LIFETIME(0)",
    "CREATE DICTIONARY d (a UInt64) PRIMARY KEY a SOURCE(CLICKHOUSE(TABLE 'x')) LAYOUT({n}({n} 4)) RANGE({n} {w} {n} {w}) LIFETIME(0)",
    "CREATE DICTIONARY d (a UInt64) PRIMARY KEY a SOURCE(CLICKHOUSE(TABLE 'x')) LAYOUT(FLAT()) LIFETIME(0) SETTINGS({n} = 1)",
];

/// Statements with their keywords in lower case, so that each is re-cased
/// and judged: one for each keyword slot that the ClickHouse corpus writes
/// only in upper case, where re-casing the corpus changes nothing for the
/// oracle to judge. Judged once each, they have no placeholder.
const KEYWORDS_IN_LOWER_CASE: &[&str] = &[
    "select count(all x), count(distinct y) from t",
    "select * ilike '%a%', t.* like 'b%' from t",
    "select a is not distinct from b, a is distinct from b, a regexp 'x', a similar to 'y', a not similar to 'z' from t",
    "create table t (a String collate binary, b UInt64 auto_increment, c Nullable(Int8) null, d Int8 not null) engine = Memory",
    "create table t (a Int8 settings (max_compress_block_size = 1) statistics(tdigest)) engine = MergeTree order by a",
    "select * except strict a from t",
    "create table t (a Int8, constraint c assume a > 0) engine = Memory",
    "create materialized view v refresh every 1 hour depends on a, db.b to t as select 1",
    "select a from t order by a offset 1 row fetch next 1 row only",
    "select count() filter (where a > 1) from t",
    "create table t (d Date) engine = MergeTree order by d ttl d to volume if exists 'v'",
    "select interval '1 2' day to hour",
    "select dateDiff(qq, a, b), dateDiff(yyyy, a, b), dateDiff(sql_tsi_month, a, b) from t",
    "select extract(doy from d), extract(isoyear from d), extract(century from d), extract(decade from d), extract(millennium from d) from t",
    "select 1 into outfile 'x' truncate and stdout format CSV",
    "select * from a natural join b",
    "select * from a paste join b",
    "select * from a join lateral (select 1) as l on true",
    "select x::JSON(a UInt8, skip b, skip regexp 'c') from t",
    "select overlay(s placing 'x' from 1) from t",
    "select a from t limit after a > 1 all until a > 5",
    "select a from t order by a offset 1 row",
    "select a from t order by all desc nulls first",
    "select a from t order by all asc nulls last",
    "select a from t order by a ascending with fill staleness 1",
    "create table t clone as u",
    "select now() at local, a is unknown, a is not unknown from t",
    "create table t (a Int8, projection p index a type basic with settings (index_granularity = 1)) engine = MergeTree order by a",
    "create materialized view v refresh every 1 hour append to t as select 1",
    "create materialized view v refresh after 1 hour append to t as select 1",
    "create materialized view v to t definer = default sql security definer as select 1",
    "create materialized view v to t sql security invoker as select 1",
    "create materialized view v to t sql security none as select 1",
    "select * from t sample 1 / 10 offset 1 / 2 where 1",
    "select all a from t",
    "select 1 settings max_threads = default",
    "select * from t stream bounded unordered watermark for ts as ts",
    "select * from t stream cursor {{'0': {{'block_number': 10}}}}",
    "create table t to inner uuid '00000000-0000-0000-0000-000000000001' engine = SharedSet",
    "create materialized view v to inner uuid '00000000-0000-0000-0000-000000000001' engine = Memory as select 1",
    "select top 10 with ties a from t order by a",
    "create table t (d Date) engine = MergeTree order by d ttl d delete where 1, d + 1 to disk 'a', d + 2 recompress codec(ZSTD(1))",
    "create table t (a Int8) engine = MergeTree order by a unique key a",
    "select * from (values (1), (2))",
    "create table ts engine = TimeSeries data db.d tags inner engine = Memory metrics inner uuid '00000000-0000-0000-0000-000000000001'",
    "create table ts engine = TimeSeries recent samples inner columns (id UUID) time ranges engine = MergeTree order by id metric families m samples db.s",
    "with recursive r as (select 1) select * from r",
    "select * from a as x local join b on 1",
    "create materialized view v refresh every 1 hour append incremental to t as select 1",
    "create materialized view v refresh every 1 hour to inner uuid '00000000-0000-0000-0000-000000000001' engine = Memory as select 1",
    "insert into table t (a, b) settings max_threads = 1 values (1, 2)",
    "insert into t from infile 'f.csv.gz' compression 'gzip' settings max_threads = 1 format CSV",
    "insert into function file('f.csv', 'CSV') partition by a select 1 as a",
    "insert into t (* except (b), t.* except c, columns('x')) values (1)",
    "insert into t select * from input('a UInt8') format CSV",
    "with x as (select 1) insert into t select * from x",
    "insert into t format Values (1)",
    "drop temporary table if exists t on cluster c sync",
    "drop table if exists if empty a, b no delay",
    "drop view if exists v",
    "drop dictionary if exists d",
    "drop database if exists d on cluster c sync",
    "drop function if exists f on cluster c",
    "drop named collection if exists c",
    "drop index if exists i on t",
    "drop workload if exists w",
    "drop resource if exists r",
    "drop handler if exists h on cluster c",
    "detach table if exists t on cluster c permanently sync",
    "detach view v",
    "detach dictionary d",
    "detach database d",
    "truncate table if exists t on cluster c",
    "truncate temporary table t",
    "truncate database if exists d",
    "truncate all tables from if exists d not ilike 'x%'",
    "truncate tables from d like 'x%'",
    "undrop table t uuid '00000000-0000-0000-0000-000000000001' on cluster c",
    "attach table if not exists t uuid '00000000-0000-0000-0000-000000000001' as not replicated on cluster c",
    "attach table t as replicated",
    "attach dictionary if not exists d on cluster c",
    "attach database if not exists d on cluster c",
    "rename table if exists a to b, c to d on cluster c",
    "rename dictionary a to b",
    "rename database if exists a to b on cluster c",
    "exchange tables a and b on cluster c",
    "exchange dictionaries a and b",
    "optimize table t on cluster c partition id '1' final deduplicate by * except (a)",
    "optimize table t partition all force cleanup",
    "optimize table t partition tuple() dry run parts 'a', 'b' final",
    "optimize table t deduplicate by columns('a') except b",
    "optimize table t final format Null settings optimize_throw_if_noop = 1",
    "drop table t into outfile 'x' format Null",
    "set max_threads = default, send_logs_level = 'fatal'",
    "set time zone 'UTC'",
    "use database d",
    "alter temporary table t on cluster c add column if not exists a Int8 after b, modify column a remove ttl",
    "alter table t alter column b type Int8 first, modify column a modify setting x = 1",
    "alter table t rename column if exists b to c, modify column a reset setting x",
    "alter table t clear column a in partition 1, comment column if exists b 'x', materialize column c",
    "alter table t modify order by a, modify sample by b, remove sample by",
    "alter table t remove ttl, materialize ttl in partition 1, rewrite parts in partition 1",
    "alter table t apply deleted mask in partition 1, apply patches",
    "alter table t add index if not exists i a type minmax granularity 1 first, drop index if exists i",
    "alter table t clear index i in partition 1, materialize index if exists i",
    "alter table t add projection p (select a) after q, modify projection if exists p (select b)",
    "alter table t clear projection p in partition 1, materialize projection p, drop projection if exists p",
    "alter table t add constraint if not exists c check 1, modify constraint c assume 1, drop constraint if exists c",
    "alter table t modify statistics a type uniq, add statistics if not exists a type tdigest",
    "alter table t clear statistics all, materialize statistics if exists a in partition 1, drop statistics if exists a",
    "alter table t delete in partition 1, 2 where 1, update a = 1 in partition 1 where 1",
    "alter table t drop detached partition all, drop detached part 'x', drop part 'y', forget partition 1",
    "alter table t detach part 'x', attach part 'y' from 'z', attach partition id '1' from u",
    "alter table t move partition 1 to disk 'd', move part 'x' to volume 'v', move part 'y' to shard 'z'",
    "alter table t fetch partition 1 from 'x', fetch part 'y' from 'z'",
    "alter table t freeze partition 1 with name 'x', unfreeze with name 'x', freeze",
    "alter table mv modify sql security invoker, modify definer = u, modify refresh every 1 day",
    "alter table t modify comment 'x', modify column a add enum values ('b' = 2)",
    "alter table t (drop column a), (drop column b)",
    "alter table t execute expire_snapshots('x')",
    "alter database d on cluster c modify comment 'x'",
    "delete from db.t on cluster c in partition id '1' where 1 settings mutations_sync = 1",
    "update db.t on cluster c set a = 1 in partition all where 1 settings mutations_sync = 1",
    "create temporary view if not exists v as select 1",
    "create view if not exists v on cluster c definer = current_user sql security definer as select 1 comment 'x'",
    "create database if not exists d uuid '00000000-0000-0000-0000-000000000001' on cluster c engine = Atomic settings a = 1 comment 'x'",
    "create or replace function f on cluster c as x -> x",
    "create function if not exists f as x -> x",
    "create or replace dictionary if not exists d on cluster c (a UInt64 default 0 expression 1 hierarchical bidirectional injective is_object_id) primary key a source(clickhouse(table 'x')) layout(flat()) lifetime(min 0 max 1) range(min a max a) settings(x = 1) comment 'c'",
];

/// Statements where a word in a name position is rightly a keyword: a
/// column may leave out its type before `AUTO_INCREMENT`, which is then no
/// type's name.
const NAME_EXCEPTIONS: &[&str] = &[
    "create table t (a auto_increment) engine = Memory",
    "alter table t add column a auto_increment",
];

/// Words that are literals rather than keywords, or both.
const LITERAL_WORDS: [&str; 7] = ["true", "false", "null", "inf", "infinity", "nan", "unknown"];

/// `template` with `{w}` and `{n}` replaced by `word`, and `{{` and `}}` by
/// braces; and where the word went, as byte ranges, all of them and those
/// of `{n}`.
fn fill(template: &str, word: &str) -> (String, Vec<Range<usize>>, Vec<Range<usize>>) {
    let mut out = String::new();
    let mut at = Vec::new();
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(c) = rest.chars().next() {
        let placeholder = ["{w}", "{n}"]
            .into_iter()
            .find_map(|p| rest.strip_prefix(p).map(|tail| (p, tail)));
        if let Some((placeholder, tail)) = placeholder {
            let range = out.len()..out.len() + word.len();
            if placeholder == "{n}" {
                names.push(range.clone());
            }
            at.push(range);
            out.push_str(word);
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("{{").or_else(|| rest.strip_prefix("}}")) {
            out.push(c);
            rest = tail;
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    (out, at, names)
}

/// A position filled with one word.
struct Case {
    text: String,
    /// Where the word is in `text`.
    word: Vec<Range<usize>>,
    /// Where the word stands for a name.
    names: Vec<Range<usize>>,
    /// The index of the template in [`POSITIONS`], then [`STATEMENTS`].
    position: usize,
    /// Whether the template is a whole statement.
    statement: bool,
    /// Whether the oracle judges the case. A word in upper case is only
    /// parsed and verified: re-casing cannot change it, and the template's
    /// own keywords are judged with the other cases of the word.
    judged: bool,
}

fn cases() -> Vec<Case> {
    let words = Keyword::ALL
        .iter()
        .map(|k| k.as_str().to_ascii_lowercase())
        .chain(LITERAL_WORDS.map(str::to_owned));
    let templates = POSITIONS
        .iter()
        .map(|t| (t, false))
        .chain(STATEMENTS.iter().map(|t| (t, true)))
        .chain(KEYWORDS_IN_LOWER_CASE.iter().map(|t| (t, true)));
    let templates: Vec<(&&str, bool)> = templates.collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut cases = Vec::new();
    for word in words {
        let mut capitalised = word.clone();
        capitalised[..1].make_ascii_uppercase();
        let upper = word.to_ascii_uppercase();
        for (variant, judged) in [(word, true), (capitalised, true), (upper, false)] {
            for (position, (template, statement)) in templates.iter().enumerate() {
                let (text, word, names) = fill(template, &variant);
                if seen.insert(text.clone()) {
                    cases.push(Case {
                        text,
                        word,
                        names,
                        position,
                        statement: *statement,
                        judged,
                    });
                }
            }
        }
    }
    cases
}

/// The case parsed and re-cased: a failure, or the re-cased text.
fn check(case: &Case) -> (Option<String>, Option<String>) {
    let text = case.text.as_str();
    let entry = if case.statement {
        Entry::File
    } else {
        Entry::Expression
    };
    let parse = parse_as(text, Mode::Sql, entry);
    let recased = parse.recased();
    if let Err(e) = verify(text, &recased, Mode::Sql, parse.facts()) {
        return (
            Some(format!("{text}: re-casing fails verification: {e}")),
            None,
        );
    }
    let name_slot = parse
        .syntax()
        .descendants_with_tokens()
        .filter_map(rowan::NodeOrToken::into_token)
        .any(|t| {
            matches!(t.kind(), SyntaxKind::Keyword | SyntaxKind::LiteralWord)
                && case
                    .names
                    .iter()
                    .any(|r| r.start == usize::from(t.text_range().start()))
        });
    if name_slot && !NAME_EXCEPTIONS.iter().any(|e| e.eq_ignore_ascii_case(text)) {
        return (Some(format!("{text}: a name is a keyword slot")), None);
    }
    (None, Some(recased))
}

#[test]
fn keyword_slots_keep_what_clickhouse_reads() {
    let program = std::env::var_os("CHAIRLIFT_CLICKHOUSE_FORMAT")
        .unwrap_or_else(|| "clickhouse-format".into());
    let oracle = Oracle::new(program);
    if !oracle.is_available() {
        assert!(
            std::env::var_os("CHAIRLIFT_REQUIRE_CORPUS").is_none_or(|v| v.is_empty()),
            "clickhouse-format is unavailable, and CHAIRLIFT_REQUIRE_CORPUS is set"
        );
        eprintln!("skipping: clickhouse-format is unavailable");
        return;
    }
    let cases = cases();
    // Parsing and verifying are independent per case; spread them over the
    // CPUs, since there are many.
    let jobs = std::thread::available_parallelism().map_or(1, usize::from);
    let chunk = cases.len().div_ceil(jobs);
    let checked: Vec<(Option<String>, Option<String>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = cases
            .chunks(chunk)
            .map(|chunk| scope.spawn(move || chunk.iter().map(check).collect::<Vec<_>>()))
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("no panic"))
            .collect()
    });
    let mut failures = Vec::new();
    let mut judged = Vec::new();
    let mut template_judged =
        vec![false; POSITIONS.len() + STATEMENTS.len() + KEYWORDS_IN_LOWER_CASE.len()];
    for (case, (failure, recased)) in cases.iter().zip(checked) {
        failures.extend(failure);
        let Some(recased) = recased else {
            continue;
        };
        let text = case.text.as_str();
        // Only a re-cased word is news in an expression; its template's own
        // keywords are judged once. A statement is judged whenever it
        // changes: what the word is can change how the rest parses.
        let word_recased = case
            .word
            .iter()
            .any(|r| recased[r.clone()] != text[r.clone()]);
        let judge_it = if !case.judged {
            false
        } else if case.statement {
            recased != text
        } else {
            word_recased || (recased != text && !template_judged[case.position])
        };
        if judge_it {
            template_judged[case.position] = true;
            judged.push((case, recased));
        }
    }
    let text = |case: &Case, text: &str| {
        if case.statement {
            text.to_owned()
        } else {
            format!("SELECT 0, {text}")
        }
    };
    // The inputs first, then the re-cased texts of those the oracle
    // accepts: a rejected input leaves nothing to judge, and each rejected
    // text costs the oracle a process of its own.
    let inputs: Vec<String> = judged
        .iter()
        .map(|(case, _)| text(case, &case.text))
        .collect();
    let refs: Vec<&str> = inputs.iter().map(String::as_str).collect();
    let input_verdicts = oracle.canonical(&refs);
    let accepted: Vec<usize> = (0..judged.len())
        .filter(|&i| matches!(input_verdicts[i], Verdict::Accepted(_)))
        .collect();
    let outputs: Vec<String> = accepted
        .iter()
        .map(|&i| text(judged[i].0, &judged[i].1))
        .collect();
    let refs: Vec<&str> = outputs.iter().map(String::as_str).collect();
    let mut output_verdicts = oracle.canonical(&refs).into_iter();
    let verdicts: Vec<[Verdict; 2]> = input_verdicts
        .into_iter()
        .map(|input| {
            let output = if matches!(input, Verdict::Accepted(_)) {
                output_verdicts
                    .next()
                    .expect("a verdict per accepted input")
            } else {
                input.clone()
            };
            [input, output]
        })
        .collect();
    let mut name_cases = 0;
    let mut name_cases_same = 0;
    for ((case, recased), pair) in judged.iter().zip(&verdicts) {
        let judgement = judge(&pair[0], &pair[1]);
        if !case.names.is_empty() {
            name_cases += 1;
            name_cases_same += usize::from(judgement == Judgement::Same);
        }
        let case = case.text.as_str();
        match judgement {
            Judgement::Same | Judgement::Unjudgeable(_) => {}
            Judgement::Differs { input, output } => failures.push(format!(
                "{case} re-cased to {recased}: the oracle prints {:?} and {:?}",
                input.trim(),
                output.trim()
            )),
            Judgement::OutputRejected(why) => failures.push(format!(
                "{case} re-cased to {recased}: the oracle rejects it: {}",
                why.message
            )),
        }
    }
    eprintln!(
        "{} cases, {} re-cased and judged, {} failures",
        cases.len(),
        judged.len(),
        failures.len()
    );
    // The statements with names in them are lower case, so all are judged;
    // most must be ones ClickHouse accepts, or the names are not tested.
    assert!(
        name_cases_same * 10 > name_cases * 9,
        "{name_cases_same} of {name_cases} statements with names judged the same"
    );
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures
            .iter()
            .take(200)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
