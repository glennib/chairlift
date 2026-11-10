//! The parser on arbitrary input: no panic, the tree holds the input
//! exactly, no node starts or ends with trivia, re-casing the keyword slots
//! passes the verifier, and the re-cased text parses to the same tree shape.
//!
//! The first byte picks the mode and the entry (see
//! `chairlift::syntax::Entry`); the rest is the text.

#![no_main]

use chairlift::lexer::Mode;
use chairlift::syntax::Entry;
use chairlift::syntax::SyntaxKind;
use chairlift::syntax::SyntaxNode;
use chairlift::syntax::parse_as;
use libfuzzer_sys::fuzz_target;

fn kinds(root: &SyntaxNode) -> Vec<SyntaxKind> {
    root.descendants_with_tokens().map(|e| e.kind()).collect()
}

fuzz_target!(|data: &[u8]| {
    let Some((&first, rest)) = data.split_first() else {
        return;
    };
    let Ok(source) = std::str::from_utf8(rest) else {
        return;
    };
    let mode = if first & 1 == 0 { Mode::Sql } else { Mode::Jinja };
    let entry = match (first >> 1) % 4 {
        0 => Entry::File,
        1 => Entry::SelectListProbe,
        2 => Entry::Expression,
        _ => Entry::DataType,
    };
    let parse = parse_as(source, mode, entry);
    let root = parse.syntax();
    assert_eq!(root.text(), source, "lossless");
    for node in root.descendants().skip(1) {
        let first = node.first_token().map(|t| t.kind());
        let last = node.last_token().map(|t| t.kind());
        assert!(
            first.is_some_and(|k| !k.is_trivia()) && last.is_some_and(|k| !k.is_trivia()),
            "{:?} starts or ends with trivia",
            node.kind()
        );
    }
    let recased = parse.recased();
    if let Err(e) = chairlift::verify::verify(source, &recased, mode, parse.facts()) {
        panic!("re-casing fails verification: {e}");
    }
    let again = parse_as(&recased, mode, entry);
    assert_eq!(
        kinds(&again.syntax()),
        kinds(&root),
        "the parse depends on case"
    );
});
