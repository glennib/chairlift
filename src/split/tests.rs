use super::*;

/// Each statement's text, with `|` before its data and a `;` marker for the
/// terminator, so a test reads like the input.
fn statements(source: &str, mode: Mode) -> Vec<String> {
    let split = split(source, mode);
    check_invariants(source, &split);
    split
        .statements()
        .iter()
        .map(|s| {
            let mut text = split.text(s, source).to_owned();
            if let Some(data) = s.data {
                let token = split.tokens()[data.token];
                let tag = match data.format {
                    DataFormat::Values => "values",
                    DataFormat::Other => "data",
                };
                text = format!("{text} <{tag}:{:?}>", token.text(source));
            }
            if s.terminator.is_none() {
                text.push_str(" <no ;>");
            }
            text
        })
        .collect()
}

fn check_invariants(source: &str, split: &Split) {
    let tokens = split.tokens();
    let mut pos = 0;
    for (i, token) in tokens.iter().enumerate() {
        assert_eq!(token.start(), pos, "{token:?} in {source:?}");
        assert!(token.end() > token.start(), "{token:?} is empty");
        if i > 0 {
            assert!(
                !(token.kind() == TokenKind::Whitespace
                    && tokens[i - 1].kind() == TokenKind::Whitespace),
                "adjacent whitespace at {i} in {source:?}"
            );
        }
        pos = token.end();
    }
    assert_eq!(pos, source.len());
    let mut after = 0;
    for statement in split.statements() {
        let range = &statement.tokens;
        assert!(range.start >= after && range.start < range.end);
        assert!(tokens[range.start].kind().is_significant());
        assert!(tokens[range.end - 1].kind().is_significant());
        assert!(
            tokens[after..range.start]
                .iter()
                .all(|t| t.kind().is_trivia())
        );
        if let Some(t) = statement.terminator {
            assert_eq!(t, range.end - 1);
            assert_eq!(tokens[t].kind(), TokenKind::Semicolon);
        }
        if let Some(data) = statement.data {
            assert!(range.contains(&data.token));
            assert_eq!(tokens[data.token].kind(), TokenKind::InsertData);
        }
        after = range.end;
    }
    assert!(tokens[after..].iter().all(|t| t.kind().is_trivia()));
    let data_tokens = tokens
        .iter()
        .filter(|t| t.kind() == TokenKind::InsertData)
        .count();
    let with_data = split
        .statements()
        .iter()
        .filter(|s| s.data.is_some())
        .count();
    assert_eq!(data_tokens, with_data);
}

fn sql(source: &str) -> Vec<String> {
    statements(source, Mode::Sql)
}

#[test]
fn semicolons_split() {
    assert_eq!(
        sql("SELECT 1; SELECT 2;\nSELECT 3"),
        ["SELECT 1;", "SELECT 2;", "SELECT 3 <no ;>"]
    );
    assert_eq!(sql(""), Vec::<String>::new());
    assert_eq!(sql("  -- only a comment\n"), Vec::<String>::new());
}

#[test]
fn semicolons_inside_tokens_do_not_split() {
    assert_eq!(
        sql("SELECT ';', `;`, \";\", $$;$$ /* ; */ -- ;\n;"),
        ["SELECT ';', `;`, \";\", $$;$$ /* ; */ -- ;\n;"]
    );
    assert_eq!(
        statements("SELECT {{ ';' }}, '{% x %};'; SELECT 2", Mode::Jinja),
        ["SELECT {{ ';' }}, '{% x %};';", "SELECT 2 <no ;>"]
    );
}

#[test]
fn a_semicolon_inside_brackets_still_splits() {
    // ClickHouse's client skips to the next `;` whatever the nesting when a
    // statement does not parse.
    assert_eq!(
        sql("SELECT (1; SELECT 2)"),
        ["SELECT (1;", "SELECT 2) <no ;>"]
    );
}

#[test]
fn stray_semicolons_are_empty_statements() {
    let source = "SELECT 1;; ;";
    let split = split(source, Mode::Sql);
    let empty: Vec<bool> = split.statements().iter().map(Statement::is_empty).collect();
    assert_eq!(empty, [false, true, true]);
}

#[test]
fn trivia_around_statements_is_outside_them() {
    assert_eq!(
        sql("-- head\nSELECT 1 /* in */ + 2; -- tail\n\n/* x */ SELECT 2"),
        ["SELECT 1 /* in */ + 2;", "SELECT 2 <no ;>"]
    );
    assert_eq!(
        statements("{% if a %}\nSELECT 1;\n{% endif %}\n", Mode::Jinja),
        ["SELECT 1;"]
    );
}

#[test]
fn values_data_runs_to_the_semicolon() {
    assert_eq!(
        sql("INSERT INTO t VALUES (1, 'a;b'), (2, ';');\nSELECT 1;"),
        [
            "INSERT INTO t VALUES (1, 'a;b'), (2, ';'); <values:\"(1, 'a;b'), (2, ';')\">",
            "SELECT 1;"
        ]
    );
    assert_eq!(
        sql("insert into t values\n  (1),  \n  -- two\n  (2)  \n;"),
        [
            "insert into t values\n  (1),  \n  -- two\n  (2)  \n; <values:\"(1),  \\n  -- two\\n  (2)\">"
        ]
    );
    assert_eq!(
        sql("INSERT INTO t VALUES (1)"),
        ["INSERT INTO t VALUES (1) <values:\"(1)\"> <no ;>"]
    );
    assert_eq!(
        sql("INSERT INTO t VALUES /* c */ (1);"),
        ["INSERT INTO t VALUES /* c */ (1); <values:\"(1)\">"]
    );
}

#[test]
fn values_without_rows() {
    assert_eq!(sql("INSERT INTO t VALUES;"), ["INSERT INTO t VALUES;"]);
    assert_eq!(sql("INSERT INTO t VALUES"), ["INSERT INTO t VALUES <no ;>"]);
}

#[test]
fn values_with_a_lexer_error_keeps_its_tokens() {
    let source = "INSERT INTO t VALUES ('a);\nSELECT 1;";
    let split = split(source, Mode::Sql);
    check_invariants(source, &split);
    assert_eq!(split.statements().len(), 1);
    assert_eq!(split.statements()[0].data, None);
    assert_eq!(
        split.first_error().map(Token::kind),
        Some(TokenKind::ErrorSingleQuoteIsNotClosed)
    );
}

#[test]
fn format_data_runs_to_a_blank_line() {
    assert_eq!(
        sql("INSERT INTO t FORMAT CSV\n1,\"a;\n\\N,'\n\nSELECT 1;"),
        [
            "INSERT INTO t FORMAT CSV\n1,\"a;\n\\N,' <data:\"1,\\\"a;\\n\\\\N,'\"> <no ;>",
            "SELECT 1;"
        ]
    );
    // Error tokens the lexer would make of the data are not tokens.
    let split = split("INSERT INTO t FORMAT TSV\n\\N\t'x\n", Mode::Sql);
    assert_eq!(split.first_error(), None);
}

#[test]
fn format_data_starts_after_one_line_break() {
    assert_eq!(
        sql("INSERT INTO t FORMAT CSV 1,2"),
        ["INSERT INTO t FORMAT CSV 1,2 <data:\"1,2\"> <no ;>"]
    );
    assert_eq!(
        sql("INSERT INTO t FORMAT CSV \t\r\n  1,2\n"),
        ["INSERT INTO t FORMAT CSV \t\r\n  1,2\n <data:\"  1,2\\n\"> <no ;>"]
    );
    // A comment after the name is data.
    assert_eq!(
        sql("INSERT INTO t FORMAT CSV -- c\n1"),
        ["INSERT INTO t FORMAT CSV -- c\n1 <data:\"-- c\\n1\"> <no ;>"]
    );
    // A blank line right after the name leaves empty data, and the next
    // statement starts there.
    assert_eq!(
        sql("INSERT INTO t FORMAT CSV\n\n\n1,2\n"),
        ["INSERT INTO t FORMAT CSV <no ;>", "1,2 <no ;>"]
    );
    assert_eq!(
        sql("INSERT INTO t FORMAT CSV\n\n1,2\n\n"),
        ["INSERT INTO t FORMAT CSV\n\n1,2 <data:\"\\n1,2\"> <no ;>"]
    );
}

#[test]
fn format_data_ends_only_at_two_line_feeds() {
    // `\r\n\r\n` is not `\n\n`: clickhouse-client looks for the latter only.
    assert_eq!(
        sql("INSERT INTO t FORMAT CSV\r\n1\r\n\r\nSELECT 1;"),
        ["INSERT INTO t FORMAT CSV\r\n1\r\n\r\nSELECT 1; <data:\"1\\r\\n\\r\\nSELECT 1;\"> <no ;>"]
    );
}

#[test]
fn format_without_data() {
    assert_eq!(
        sql("INSERT INTO t FORMAT CSV;"),
        ["INSERT INTO t FORMAT CSV;"]
    );
    assert_eq!(
        sql("INSERT INTO t FORMAT CSV"),
        ["INSERT INTO t FORMAT CSV <no ;>"]
    );
    assert_eq!(
        sql("INSERT INTO t FROM INFILE 'f.csv' FORMAT CSV; SELECT 1"),
        [
            "INSERT INTO t FROM INFILE 'f.csv' FORMAT CSV;",
            "SELECT 1 <no ;>"
        ]
    );
    assert_eq!(
        sql("INSERT INTO t SELECT 1 FORMAT CSV;\nSELECT 2;"),
        ["INSERT INTO t SELECT 1 FORMAT CSV;", "SELECT 2;"]
    );
    assert_eq!(sql("SELECT 1 FORMAT CSV;"), ["SELECT 1 FORMAT CSV;"]);
}

#[test]
fn select_from_input_reads_data() {
    assert_eq!(
        sql("INSERT INTO t SELECT * FROM input('a Int8') FORMAT CSV\n1\n2\n\nSELECT 1;"),
        [
            "INSERT INTO t SELECT * FROM input('a Int8') FORMAT CSV\n1\n2 <data:\"1\\n2\"> <no ;>",
            "SELECT 1;"
        ]
    );
}

#[test]
fn format_values_uses_the_values_rule() {
    assert_eq!(
        sql("INSERT INTO t FORMAT Values (1),\n(2);"),
        ["INSERT INTO t FORMAT Values (1),\n(2); <values:\"(1),\\n(2)\">"]
    );
    // clickhouse-client compares the name exactly.
    assert_eq!(
        sql("INSERT INTO t FORMAT values (1);\n\n"),
        ["INSERT INTO t FORMAT values (1); <data:\"(1);\"> <no ;>"]
    );
}

#[test]
fn insert_heads() {
    let values = |head: &str| {
        let source = format!("{head} (1);");
        let split = split(&source, Mode::Sql);
        split.statements()[0]
            .data
            .map(|d| split.tokens()[d.token].text(&source).to_owned())
    };
    let one = Some("(1)".to_owned());
    assert_eq!(values("INSERT INTO values VALUES"), one);
    assert_eq!(values("INSERT INTO db.t (a, b) VALUES"), one);
    assert_eq!(values("INSERT INTO `db`.\"t\"(a) VALUES"), one);
    assert_eq!(values("INSERT INTO TABLE t VALUES"), one);
    assert_eq!(values("INSERT INTO FUNCTION remote('h', db.t) VALUES"), one);
    assert_eq!(
        values("INSERT INTO FUNCTION s3('u') PARTITION BY rand() % 2 VALUES"),
        one
    );
    assert_eq!(
        values("INSERT INTO t SETTINGS async_insert = 1 VALUES"),
        one
    );
    assert_eq!(values("insert into t values"), one);
    assert_eq!(values("WITH 1 AS x INSERT INTO t VALUES"), one);
    assert_eq!(values("EXPLAIN SYNTAX INSERT INTO t VALUES"), one);
    assert_eq!(values("INSERT INTO t SELECT * FROM values"), None);
    assert_eq!(
        values("INSERT INTO t (SELECT 1) UNION ALL SELECT * FROM values"),
        None
    );
    assert_eq!(values("INSERT INTO t FROM t2 VALUES"), None);
    assert_eq!(values("CREATE TABLE t AS SELECT * FROM values"), None);
    assert_eq!(values("WITH x AS (SELECT 1) SELECT * FROM values"), None);
    assert_eq!(values("SELECT * FROM t WHERE INSERT INTO t VALUES"), None);
}

#[test]
fn jinja_in_inserts() {
    assert_eq!(
        statements(
            "INSERT INTO {{ t }} {% if a %} VALUES {% endif %} ({{ x }}), (2);",
            Mode::Jinja
        ),
        [
            "INSERT INTO {{ t }} {% if a %} VALUES {% endif %} ({{ x }}), (2); <values:\"({{ x }}), (2)\">"
        ]
    );
}

#[test]
fn lexing_restarts_after_data() {
    // After the data, `.5` begins a query, where ClickHouse reads it as a
    // number whatever came before.
    let source = "INSERT INTO t FORMAT CSV x\n\n.5";
    let split = split(source, Mode::Sql);
    let last = split.tokens().last().unwrap();
    assert_eq!((last.kind(), last.text(source)), (TokenKind::Number, ".5"));
}

#[test]
fn template_data_ends_at_a_blank_line_outside_tags() {
    let source = "INSERT INTO t FORMAT CSV\n{{ '\n\n' }},1\n\nSELECT 1";
    assert_eq!(
        statements(source, Mode::Jinja),
        [
            "INSERT INTO t FORMAT CSV\n{{ '\n\n' }},1 <data:\"{{ '\\n\\n' }},1\"> <no ;>",
            "SELECT 1 <no ;>"
        ]
    );
    // As plain SQL the braces are text.
    assert_eq!(
        sql(source)[0],
        "INSERT INTO t FORMAT CSV\n{{ ' <data:\"{{ '\"> <no ;>"
    );
}

#[test]
fn an_unclosed_tag_in_template_data_is_an_error() {
    let source = "INSERT INTO t FORMAT CSV\n1,{% raw %}x\n\nSELECT 1";
    let split = split(source, Mode::Jinja);
    let error = split.first_error().unwrap();
    assert_eq!(error.kind(), TokenKind::ErrorJinjaTagIsNotClosed);
    assert_eq!(error.text(source), "{% raw %}x\n\nSELECT 1");
    assert_eq!(super::split(source, Mode::Sql).first_error(), None);
}
