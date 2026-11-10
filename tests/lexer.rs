//! The lexer's edge cases, from the ClickHouse lexer checklist in
//! `docs/spikes/2026-11-10-clickhouse-internals-and-corpus.md`.
//!
//! The expectations are written out by hand. When the fetched ClickHouse
//! source and clang are available, the SQL cases are also lexed by
//! ClickHouse's own lexer, which must agree.

#[path = "support/clickhouse_lexer.rs"]
mod clickhouse_lexer;
#[path = "support/corpus.rs"]
mod corpus;

use chairlift::lexer::TokenKind;
use chairlift::lexer::TokenKind::*;
use chairlift::lexer::tokenize;

struct Case {
    input: &'static str,
    expected: &'static [(TokenKind, &'static str)],
}

/// `sql "input" => [Kind "text", …];`
macro_rules! cases {
    ($(sql $input:literal => [$($kind:ident $text:literal),* $(,)?];)*) => {
        &[$(Case { input: $input, expected: &[$(($kind, $text)),*] }),*]
    };
}

const CASES: &[Case] = cases! {
    // Comments
    sql "1--1" => [Number "1", LineComment "--1"];
    sql "-- c\nx" => [LineComment "-- c", Whitespace "\n", BareWord "x"];
    sql "// c" => [LineComment "// c"];
    sql "# c" => [LineComment "# c"];
    sql "#!/usr/bin/env clickhouse" => [LineComment "#!/usr/bin/env clickhouse"];
    sql "#x" => [Error "#", BareWord "x"];
    sql "#\tc" => [Error "#", Whitespace "\t", BareWord "c"];
    sql "#" => [Error "#"];
    sql "/* a /* b */ c */x" => [BlockComment "/* a /* b */ c */", BareWord "x"];
    sql "/* a /* b */" => [ErrorMultilineCommentIsNotClosed "/* a /* b */"];
    sql "/*/" => [ErrorMultilineCommentIsNotClosed "/*/"];
    sql "a/b" => [BareWord "a", Slash "/", BareWord "b"];

    // Numbers and the leading `.`
    sql "(1).5" => [OpeningRoundBracket "(", Number "1", ClosingRoundBracket ")", Dot ".", Number "5"];
    sql "[1].5" => [OpeningSquareBracket "[", Number "1", ClosingSquareBracket "]", Dot ".", Number "5"];
    sql "\"t\".5" => [QuotedIdentifier "\"t\"", Dot ".", Number "5"];
    sql "x.1.2" => [BareWord "x", Dot ".", Number "1", Dot ".", Number "2"];
    sql "t.1e5" => [BareWord "t", Dot ".", BareWord "1e5"];
    sql "1 .5" => [Number "1", Whitespace " ", Dot ".", Number "5"];
    sql ", .5" => [Comma ",", Whitespace " ", Number ".5"];
    sql "(.5e-3)" => [OpeningRoundBracket "(", Number ".5e-3", ClosingRoundBracket ")"];
    sql ".5" => [Number ".5"];
    sql ".x" => [Number ".", BareWord "x"];
    sql "x .y" => [BareWord "x", Whitespace " ", Dot ".", BareWord "y"];
    sql "1.5" => [Number "1.5"];
    sql "1." => [Number "1."];
    sql "1.e5" => [Number "1.e5"];
    sql "0x1.8p3" => [Number "0x1.8p3"];
    sql "0x1p-2" => [Number "0x1p-2"];
    sql "0xFF_FF" => [Number "0xFF_FF"];
    sql "0b101" => [Number "0b101"];
    sql "0b2" => [BareWord "0b2"];
    sql "0x" => [BareWord "0x"];
    sql "0xG" => [BareWord "0xG"];
    sql "0x_1" => [BareWord "0x_1"];
    sql "1_000_000" => [Number "1_000_000"];
    sql "1e1_0" => [Number "1e1_0"];
    sql "1__0" => [BareWord "1__0"];
    sql "1_" => [BareWord "1_"];
    sql "1_e5" => [BareWord "1_e5"];
    sql "1e5" => [Number "1e5"];
    sql "1e" => [BareWord "1e"];
    sql "1e+" => [Number "1e", Plus "+"];
    sql "1e+ " => [Number "1e+", Whitespace " "];
    sql "1e+x" => [ErrorWrongNumber "1e+x"];
    sql "1.5abc" => [ErrorWrongNumber "1.5abc"];
    sql "1abc" => [BareWord "1abc"];
    sql "-1" => [Minus "-", Number "1"];
    sql "\u{2212}1" => [Minus "\u{2212}", Number "1"];

    // Strings and quoted identifiers
    sql "'it''s'" => [StringLiteral "'it''s'"];
    sql "'it\\'s'" => [StringLiteral "'it\\'s'"];
    sql "'a;b'" => [StringLiteral "'a;b'"];
    sql "'abc" => [ErrorSingleQuoteIsNotClosed "'abc"];
    sql "'abc\\" => [ErrorSingleQuoteIsNotClosed "'abc\\"];
    sql "‘str’ x" => [StringLiteral "‘str’", Whitespace " ", BareWord "x"];
    sql "‘it\\’s’" => [StringLiteral "‘it\\’", BareWord "s", Error "’"];
    sql "“id”" => [QuotedIdentifier "“id”"];
    sql "‘abc" => [ErrorSingleQuoteIsNotClosed "‘abc"];
    sql "‘a" => [Error "‘", BareWord "a"];
    sql "`a``b`" => [QuotedIdentifier "`a``b`"];
    sql "\"a\\\"b\"" => [QuotedIdentifier "\"a\\\"b\""];
    sql "\"ab" => [ErrorDoubleQuoteIsNotClosed "\"ab"];
    sql "`ab" => [ErrorBackQuoteIsNotClosed "`ab"];
    sql "x'4142'" => [StringLiteral "x'4142'"];
    sql "X'4142'" => [StringLiteral "X'4142'"];
    sql "b'0101'" => [StringLiteral "b'0101'"];
    sql "x'zz' 1" => [ErrorSingleQuoteIsNotClosed "x'zz' 1"];
    sql "max'a'" => [BareWord "max", StringLiteral "'a'"];

    // Heredocs and `$`
    sql "$$it's$$" => [HereDoc "$$it's$$"];
    sql "$tag$a $$ b$tag$" => [HereDoc "$tag$a $$ b$tag$"];
    sql "$tag$abc" => [BareWord "$tag$abc"];
    sql "$a b$" => [BareWord "$a", Whitespace " ", BareWord "b$"];
    sql "$1" => [BareWord "$1"];
    sql "$ " => [DollarSign "$", Whitespace " "];
    sql "$" => [DollarSign "$"];
    sql "a$b" => [BareWord "a$b"];

    // Operators
    sql "{name:UInt8}" => [OpeningCurlyBrace "{", BareWord "name", Colon ":", BareWord "UInt8", ClosingCurlyBrace "}"];
    sql "{{ x }}" => [
        OpeningCurlyBrace "{", OpeningCurlyBrace "{", Whitespace " ", BareWord "x",
        Whitespace " ", ClosingCurlyBrace "}", ClosingCurlyBrace "}",
    ];
    sql "a::Int8" => [BareWord "a", DoubleColon "::", BareWord "Int8"];
    sql "x->x" => [BareWord "x", Arrow "->", BareWord "x"];
    sql "<=><=<>" => [Spaceship "<=>", LessOrEquals "<=", NotEquals "<>"];
    sql "!===" => [NotEquals "!=", Equals "=="];
    sql "!x" => [ErrorSingleExclamationMark "!", BareWord "x"];
    sql "!~*!~~*~" => [NotTildeAsterisk "!~*", NotTilde "!~", TildeAsterisk "~*", Tilde "~"];
    sql "|||>|" => [Concatenation "||", PipeOperator "|>", PipeMark "|"];
    sql "@@x@y" => [DoubleAt "@@", BareWord "x", At "@", BareWord "y"];
    sql "?:^%*" => [QuestionMark "?", Colon ":", Caret "^", Percent "%", Asterisk "*"];
    sql "\\G" => [VerticalDelimiter "\\G"];
    sql "\\x" => [Error "\\", BareWord "x"];
    sql "[]" => [OpeningSquareBracket "[", ClosingSquareBracket "]"];
    sql "a;b" => [BareWord "a", Semicolon ";", BareWord "b"];

    // Whitespace and other characters
    sql "a\u{a0}b" => [BareWord "a", Whitespace "\u{a0}", BareWord "b"];
    sql "\u{feff}SELECT 1" => [Whitespace "\u{feff}", BareWord "SELECT", Whitespace " ", Number "1"];
    sql " \t\u{2003}\n\u{3000}x" => [Whitespace " \t\u{2003}\n\u{3000}", BareWord "x"];
    sql "\u{b}\u{c}\r\n" => [Whitespace "\u{b}\u{c}\r\n"];
    sql "blåbær" => [BareWord "bl", Error "å", BareWord "b", Error "æ", BareWord "r"];
    sql "\u{1f642}" => [Error "\u{1f642}"];
};

#[test]
fn edge_cases() {
    assert!(CASES.len() >= 60, "the plan asks for at least 60 cases");
    let mut failures = Vec::new();
    for case in CASES {
        let tokens = tokenize(case.input);
        let actual: Vec<(TokenKind, &str)> = tokens
            .iter()
            .map(|t| (t.kind(), t.text(case.input)))
            .collect();
        if actual != case.expected {
            failures.push(format!(
                "{:?}\n  expected {:?}\n  actual   {actual:?}",
                case.input, case.expected
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn edge_cases_agree_with_clickhouse() {
    let Some(oracle) = clickhouse_lexer::Oracle::build() else {
        return;
    };
    let inputs: Vec<&str> = CASES.iter().map(|c| c.input).collect();
    let mut failures = Vec::new();
    for (input, clickhouse) in inputs.iter().zip(oracle.lex(&inputs)) {
        let chairlift = clickhouse_lexer::as_clickhouse(&tokenize(input));
        if chairlift != clickhouse {
            failures.push(format!(
                "{input:?}\n  ClickHouse {clickhouse:?}\n  chairlift  {chairlift:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
