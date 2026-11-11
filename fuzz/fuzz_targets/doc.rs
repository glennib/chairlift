//! The document printer on documents built from arbitrary bytes (see
//! `tests/support/doc_gen.rs`), in both modes: no panic, every token
//! printed once, the output verifies and has no whitespace at the end of a
//! line, and a document built the same way over the output prints it back.

#![no_main]

use chairlift::doc::PrintOptions;
use chairlift::lexer::Mode;
use chairlift::split::split;
use libfuzzer_sys::fuzz_target;

mod api {
    pub use chairlift::doc;
    pub use chairlift::lexer;
    pub use chairlift::split;
    pub use chairlift::verify;
}

#[path = "../../tests/support/doc_gen.rs"]
mod doc_gen;

use doc_gen::Bytes;
use doc_gen::Shape;

fuzz_target!(|data: &[u8]| {
    let Some((&width, data)) = data.split_first() else {
        return;
    };
    let options = PrintOptions::default()
        .with_line_width(usize::from(width))
        .with_indent_width(usize::from(width % 5));
    for mode in [Mode::Sql, Mode::Jinja] {
        let mut bytes = Bytes::new(data);
        let n = 1 + bytes.below(200);
        let source = doc_gen::source(&mut bytes, mode, n);
        if split(&source, mode).first_error().is_some() {
            continue;
        }
        let tokens = doc_gen::tokens(&source, mode);
        let start = bytes.clone();
        let doc = doc_gen::document(&mut bytes, &source, &tokens, mode, Shape::ALL);
        doc.check_tokens().expect("every token once");
        let out = doc.print(&options);
        if let Err(e) = chairlift::verify::verify(&source, &out, mode, &doc_gen::facts(&doc)) {
            panic!("{e}\n--- source:\n{source}\n--- output:\n{out}");
        }
        let trailing = doc_gen::trailing_whitespace(&out);
        assert!(trailing.is_empty(), "lines end in whitespace: {trailing:?}");
        let again_tokens = doc_gen::tokens(&out, mode);
        let again = doc_gen::document(&mut start.clone(), &out, &again_tokens, mode, Shape::ALL);
        assert_eq!(again.print(&options), out, "not idempotent");
    }
});
