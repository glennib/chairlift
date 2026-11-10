//! The verifier catches a printer bug: a toy printer that puts every token
//! on one line, once correct and once with the bug the plan names, printing
//! `-` and `-1` adjacent.

use chairlift::lexer::Mode;
use chairlift::lexer::TokenKind;
use chairlift::lexer::can_touch;
use chairlift::split::split;
use chairlift::verify::Facts;
use chairlift::verify::verify;

/// Prints the tokens with no whitespace between them, except a newline after
/// a line comment and a space where [`can_touch`] says the two would lex
/// differently, unless `glue_minus` is set and both are `-` signs.
fn print(source: &str, glue_minus: bool) -> String {
    let split = split(source, Mode::Sql);
    let mut out = String::new();
    let mut prev_significant = None;
    let mut last: Option<chairlift::lexer::Token> = None;
    for &token in split
        .tokens()
        .iter()
        .filter(|t| t.kind() != TokenKind::Whitespace)
    {
        if let Some(left) = last {
            let minus_pair = left.kind() == TokenKind::Minus && token.kind() == TokenKind::Minus;
            let touch = (glue_minus && minus_pair)
                || can_touch(
                    Mode::Sql,
                    prev_significant,
                    (left.kind(), left.text(source)),
                    (token.kind(), token.text(source)),
                );
            if left.kind() == TokenKind::LineComment {
                out.push('\n');
            } else if !touch {
                out.push(' ');
            }
            if left.kind().is_significant() {
                prev_significant = Some(left.kind());
            }
        }
        out.push_str(token.text(source));
        last = Some(token);
    }
    out
}

#[test]
fn an_injected_printer_bug_is_caught() {
    let source = "SELECT a - -1 AS b, -x - -y\nFROM t -- end\nWHERE c\n";
    let correct = print(source, false);
    assert_eq!(correct, "SELECT a- -1 AS b,-x- -y FROM t-- end\nWHERE c");
    verify(source, &correct, Mode::Sql, &Facts::new()).unwrap();

    let buggy = print(source, true);
    assert_eq!(buggy, "SELECT a--1 AS b,-x--y FROM t-- end\nWHERE c");
    let error = verify(source, &buggy, Mode::Sql, &Facts::new()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the output differs from the input at input 1:10, output 1:9: token 3 is Minus \"-\" \
         in the input and LineComment \"--1 AS b,-x--y FROM t-- end\" in the output"
    );
}
