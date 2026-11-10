//! Data types.

use crate::syntax::Entry;
use crate::syntax::test_support::tree;

fn trees(inputs: &[&str]) -> String {
    let mut out = String::new();
    for input in inputs {
        out.push_str("> ");
        out.push_str(input);
        out.push('\n');
        out.push_str(&tree(input, Entry::DataType));
        out.push('\n');
    }
    out
}

#[test]
fn data_types() {
    insta::assert_snapshot!(trees(&[
        "UInt64",
        "Nullable(String)",
        "LowCardinality(Nullable(String))",
        "Array(Array(Int8))",
        "Map(String, UInt64)",
        "Tuple(a Int8, b String)",
        "Tuple(Int8, String)",
        "Tuple()",
        "Nested(a UInt8, b Array(String))",
        "Enum8('a' = 1, 'b' = -2)",
        "Enum('a', 'b')",
        "DateTime64(3, 'UTC')",
        "Decimal(10, 2)",
        "FixedString(16)",
        "AggregateFunction(uniq, String)",
        "AggregateFunction(quantiles(0.5, 0.9), UInt64)",
        "AggregateFunction(1, sum, UInt64)",
        "SimpleAggregateFunction(anyLast, String)",
        "AggregateFunction(null, String)",
        "AggregateFunction(1, true, String)",
        "SimpleAggregateFunction(nan, Float64)",
        "JSON(max_dynamic_paths = 10, a.b UInt32, SKIP a.c, SKIP REGEXP 'x.*')",
        "Dynamic(max_types = 10)",
        "Object('json')",
        "DOUBLE PRECISION",
        "INT(11) UNSIGNED",
        "NATIONAL CHARACTER VARYING",
        "`UInt8`",
        "{{ column_type }}",
    ]));
}

#[test]
fn words_that_follow_a_missing_type_are_not_types() {
    for word in [
        "DEFAULT",
        "null",
        "Codec",
        "TTL",
        "MATERIALIZED",
        "ALIAS",
        "EPHEMERAL",
    ] {
        assert!(
            tree(word, Entry::DataType).starts_with("VerbatimStatement"),
            "{word}"
        );
    }
}
