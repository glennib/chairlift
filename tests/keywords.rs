//! Checks the generated keyword list against ClickHouse's source, read here
//! independently of `scripts/generate-keywords.sh`.

#[path = "support/corpus.rs"]
mod corpus;

use std::collections::BTreeSet;
use std::fs;

use chairlift::keyword::Keyword;

/// The keyword phrases of `APPLY_FOR_PARSER_KEYWORDS*`, in source order.
fn clickhouse_phrases(header: &str) -> Vec<String> {
    let mut phrases = Vec::new();
    let mut inside = false;
    for line in header.lines() {
        if line.starts_with("#define APPLY_FOR_PARSER_KEYWORDS") {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(rest) = line.trim_start().strip_prefix("MR_MACROS(") {
            let text = rest
                .split('"')
                .nth(1)
                .expect("an MR_MACROS entry has a quoted name");
            phrases.push(text.to_ascii_uppercase());
        }
        if !line.trim_end().ends_with('\\') {
            inside = false;
        }
    }
    phrases
}

#[test]
fn generated_list_matches_clickhouse_source() {
    let Some(clickhouse) = corpus::clickhouse() else {
        corpus::skip("the ClickHouse checkout");
        return;
    };
    let header = fs::read_to_string(clickhouse.join("src/Parsers/CommonParsers.h"))
        .expect("CommonParsers.h is readable");
    let phrases = clickhouse_phrases(&header);

    let words: BTreeSet<&str> = phrases.iter().flat_map(|p| p.split(' ')).collect();
    let generated: BTreeSet<&str> = Keyword::ALL.iter().map(|k| k.as_str()).collect();
    assert_eq!(generated, words);

    let generated_phrases: Vec<String> = Keyword::phrases()
        .iter()
        .map(|p| p.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(generated_phrases, phrases);
}
