//! Property tests of the document printer, over random documents (see
//! `support/doc_gen.rs`) and over random SQL laid out by the style's rules
//! (see `support/doc_sql.rs`).

use chairlift::doc::Builder;
use chairlift::doc::Doc;
use chairlift::doc::PrintOptions;
use chairlift::lexer::Mode;
use chairlift::lexer::TokenKind;
use chairlift::split::split;
use chairlift::verify::verify;
use proptest::prelude::*;

mod api {
    pub use chairlift::doc;
    pub use chairlift::lexer;
    pub use chairlift::split;
    pub use chairlift::verify;
}

#[path = "support/doc_gen.rs"]
mod doc_gen;
#[path = "support/doc_sql.rs"]
mod doc_sql;

use doc_gen::Bytes;
use doc_gen::Shape;

fn config() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// A source text of up to 60 words and a document over it, from `data`,
/// or `None` if the text has a lexer error, which the formatter never
/// formats.
fn generated(data: &[u8], mode: Mode) -> Option<String> {
    let mut bytes = Bytes::new(data);
    let n = 1 + bytes.below(60);
    let source = doc_gen::source(&mut bytes, mode, n);
    split(&source, mode)
        .first_error()
        .is_none()
        .then_some(source)
}

/// The document over `source` that `data` builds, as `generated` built it.
fn rebuild<'s>(
    data: &[u8],
    source: &'s str,
    tokens: &'s [chairlift::lexer::Token],
    mode: Mode,
    shape: Shape,
) -> Doc<'s> {
    let mut bytes = Bytes::new(data);
    let n = 1 + bytes.below(60);
    // Consume the bytes the source took.
    doc_gen::source(&mut bytes, mode, n);
    doc_gen::document(&mut bytes, source, tokens, mode, shape)
}

fn mode_of(jinja: bool) -> Mode {
    if jinja { Mode::Jinja } else { Mode::Sql }
}

proptest! {
    #![proptest_config(config())]

    /// Every token is printed once, in order, the output verifies (so no two
    /// tokens were glued into others, and Jinja expressions keep their
    /// spacing), no line ends in whitespace, and printing a document built
    /// the same way over the output gives the output back.
    #[test]
    fn printing_keeps_the_tokens_and_is_idempotent(
        data in prop::collection::vec(any::<u8>(), 0..800),
        width in 0usize..80,
        indent in 0usize..6,
        jinja in any::<bool>(),
    ) {
        let mode = mode_of(jinja);
        let Some(source) = generated(&data, mode) else {
            return Ok(());
        };
        let tokens = doc_gen::tokens(&source, mode);
        let doc = rebuild(&data, &source, &tokens, mode, Shape::ALL);
        prop_assert_eq!(doc.check_tokens(), Ok(()));
        let options = PrintOptions::default().with_line_width(width).with_indent_width(indent);
        let out = doc.print(&options);
        if let Err(e) = verify(&source, &out, mode, &doc_gen::facts(&doc)) {
            prop_assert!(false, "{}\n--- source:\n{}\n--- output:\n{}", e, source, out);
        }
        let trailing = doc_gen::trailing_whitespace(&out);
        prop_assert!(trailing.is_empty(), "lines end in whitespace: {:?}", trailing);
        prop_assert!(!out.starts_with('\n'), "the output starts with a blank line");
        let again_tokens = doc_gen::tokens(&out, mode);
        let again = rebuild(&data, &out, &again_tokens, mode, Shape::ALL);
        prop_assert_eq!(again.print(&options), out);
    }

    /// The output has the fewest line breaks when every group fits, and the
    /// most when none does.
    #[test]
    fn line_breaks_lie_between_the_widest_and_narrowest_prints(
        data in prop::collection::vec(any::<u8>(), 0..800),
        width in 0usize..80,
        jinja in any::<bool>(),
    ) {
        let mode = mode_of(jinja);
        let Some(source) = generated(&data, mode) else {
            return Ok(());
        };
        let tokens = doc_gen::tokens(&source, mode);
        let doc = rebuild(&data, &source, &tokens, mode, Shape::ALL);
        let lines = |width| doc.print(&PrintOptions::default().with_line_width(width)).matches('\n').count();
        let (widest, here, narrowest) = (lines(usize::MAX), lines(width), lines(0));
        prop_assert!(widest <= here && here <= narrowest, "{} {} {}", widest, here, narrowest);
    }

    /// Where every boundary between tokens can break, no line is wider than
    /// the width, unless one token and its indentation are.
    #[test]
    fn lines_fit_where_every_boundary_can_break(
        data in prop::collection::vec(any::<u8>(), 0..400),
        width in 16usize..60,
    ) {
        const WORDS: &[&str] = &["a", "bb", "ccc", "dddd", "(", ")", "f(x)", "'s'", "1"];
        let mut bytes = Bytes::new(&data);
        let n = 1 + bytes.below(80);
        let source: Vec<&str> = (0..n).map(|_| WORDS[bytes.below(WORDS.len())]).collect();
        let source = source.join(" ");
        let tokens = doc_gen::tokens(&source, Mode::Sql);
        let mut b = Builder::new(&source, &tokens, Mode::Sql);
        let mut open = Vec::new();
        let mut indents = 0;
        for (i, token) in tokens.iter().enumerate() {
            if token.kind() == TokenKind::Whitespace {
                continue;
            }
            if i > 0 {
                b.line();
            }
            match bytes.below(4) {
                0 if open.len() < 8 => open.push((b.open_group(), false)),
                1 if indents < 3 => {
                    open.push((b.open_indent(), true));
                    indents += 1;
                }
                2 => {
                    if let Some((scope, indent)) = open.pop() {
                        b.close(scope);
                        indents -= usize::from(indent);
                    }
                }
                _ => {}
            }
            b.token(i);
        }
        while let Some((scope, _)) = open.pop() {
            b.close(scope);
        }
        let doc = b.finish();
        let out = doc.print(&PrintOptions::default().with_line_width(width));
        for line in out.lines() {
            prop_assert!(line.chars().count() <= width, "{:?} is wider than {}", line, width);
        }
    }

    /// SQL laid out by the style: the token check passes, the output
    /// verifies, no line ends in whitespace, and laying out the output again
    /// gives it back.
    #[test]
    fn laid_out_sql_keeps_its_tokens_and_is_idempotent(
        seed in any::<u64>(),
        statements in 1usize..6,
        width in 20usize..120,
    ) {
        let mut rng = doc_sql::Rng::new(seed);
        let (queries, source) = doc_sql::script(&mut rng, statements);
        let tokens = doc_gen::tokens(&source, Mode::Sql);
        let doc = doc_sql::document(&queries, &source, &tokens, Mode::Sql);
        prop_assert_eq!(doc.check_tokens(), Ok(()));
        let options = PrintOptions::default().with_line_width(width);
        let out = doc.print(&options);
        if let Err(e) = verify(&source, &out, Mode::Sql, &doc_gen::facts(&doc)) {
            prop_assert!(false, "{}\n--- output:\n{}", e, out);
        }
        prop_assert!(doc_gen::trailing_whitespace(&out).is_empty());
        let again_tokens = doc_gen::tokens(&out, Mode::Sql);
        let again = doc_sql::document(&queries, &out, &again_tokens, Mode::Sql);
        prop_assert_eq!(again.print(&options), out);
    }
}
