//! Unit tests for the IR's constructs and the printer's rules.

use proptest::prelude::*;

use super::*;
use crate::split::split;
use crate::verify::Facts;
use crate::verify::verify;

mod api {
    pub use crate::doc;
    pub use crate::lexer;
    pub use crate::split;
    pub use crate::verify;
}

#[path = "../../tests/support/doc_gen.rs"]
mod doc_gen;

/// Hands out the tokens of a source other than whitespace, in order,
/// checking their text.
struct Next<'t> {
    tokens: &'t [Token],
    source: &'t str,
    at: usize,
}

impl Next<'_> {
    /// The index of the next token, which must be `text`.
    fn t(&mut self, text: &str) -> usize {
        while self.tokens[self.at].kind() == TokenKind::Whitespace {
            self.at += 1;
        }
        let i = self.at;
        assert_eq!(self.tokens[i].text(self.source), text, "token {i}");
        self.at += 1;
        i
    }
}

/// Builds a document over `source` with `build`, checks its tokens, prints
/// it at `width` with both measurements, checks they agree and that the
/// output verifies, and returns the output.
fn print_at(
    source: &str,
    mode: Mode,
    width: usize,
    build: impl FnOnce(&mut Builder<'_>, &mut Next<'_>),
) -> String {
    let split = split(source, mode);
    let tokens = split.tokens();
    let mut b = Builder::new(source, tokens, mode);
    let mut next = Next {
        tokens,
        source,
        at: 0,
    };
    build(&mut b, &mut next);
    let doc = b.finish();
    doc.check_tokens().expect("tokens");
    let options = PrintOptions::default().with_line_width(width);
    let out = doc.print(&options);
    assert_eq!(
        print::print_reference(&doc, &options),
        out,
        "the reference measurement prints otherwise"
    );
    let mut facts = Facts::new();
    for token in doc.recased() {
        facts.recase(token);
    }
    if let Err(e) = verify(source, &out, mode, &facts) {
        panic!("{e}\n--- output:\n{out}");
    }
    out
}

fn print(source: &str, build: impl FnOnce(&mut Builder<'_>, &mut Next<'_>)) -> String {
    print_at(source, Mode::Sql, 100, build)
}

/// `name(args)`: the arguments on one line if they fit, else one per line.
fn call(b: &mut Builder<'_>, n: &mut Next<'_>, name: &str, args: &[&str]) {
    b.token(n.t(name));
    b.token(n.t("("));
    b.group(|b| {
        b.indent(|b| {
            b.soft_line();
            for (k, arg) in args.iter().enumerate() {
                if k > 0 {
                    b.token(n.t(","));
                    b.line();
                }
                b.token(n.t(arg));
            }
        });
        b.soft_line();
    });
    b.token(n.t(")"));
}

#[test]
fn tokens_print_as_written_or_recased() {
    let out = print("select Nan, null, true, Foo from t", |b, n| {
        b.keyword(n.t("select"));
        b.space();
        b.literal_word(n.t("Nan"));
        b.token(n.t(","));
        b.space();
        b.literal_word(n.t("null"));
        b.token(n.t(","));
        b.space();
        b.literal_word(n.t("true"));
        b.token(n.t(","));
        b.space();
        b.token(n.t("Foo"));
        b.space();
        b.recased(n.t("from"), "FROM");
        b.space();
        b.token(n.t("t"));
    });
    assert_eq!(out, "SELECT nan, NULL, true, Foo FROM t\n");
}

#[test]
fn spaces_are_lazy_and_never_trail() {
    let out = print("a b c", |b, n| {
        b.token(n.t("a"));
        b.space();
        b.space();
        b.token(n.t("b"));
        b.space();
        b.hard_line();
        b.space();
        b.token(n.t("c"));
        b.space();
    });
    assert_eq!(out, "a b\nc\n");
}

#[test]
fn line_breaks_at_one_place_do_not_add_up() {
    let out = print("a b", |b, n| {
        b.token(n.t("a"));
        b.hard_line();
        b.line();
        b.hard_line();
        b.token(n.t("b"));
    });
    assert_eq!(out, "a\nb\n");
}

#[test]
fn a_group_fits_up_to_exactly_the_width() {
    let build = |b: &mut Builder<'_>, n: &mut Next<'_>| {
        b.group(|b| {
            b.token(n.t("aaaa"));
            b.line();
            b.token(n.t("bbbb"));
        });
    };
    assert_eq!(print_at("aaaa bbbb", Mode::Sql, 9, build), "aaaa bbbb\n");
    assert_eq!(print_at("aaaa bbbb", Mode::Sql, 8, build), "aaaa\nbbbb\n");
}

#[test]
fn soft_lines_print_nothing_in_a_flat_group() {
    let source = "f(a, b)";
    let build = |b: &mut Builder<'_>, n: &mut Next<'_>| call(b, n, "f", &["a", "b"]);
    assert_eq!(print_at(source, Mode::Sql, 7, build), "f(a, b)\n");
    assert_eq!(
        print_at(source, Mode::Sql, 6, build),
        "f(\n    a,\n    b\n)\n"
    );
}

#[test]
fn what_follows_a_group_counts_up_to_the_next_break() {
    // `f(a, b) AS alias,` is 17 columns: the arguments break at 16 although
    // `f(a, b)` alone fits.
    let source = "f(a, b) as alias, c";
    let build = |b: &mut Builder<'_>, n: &mut Next<'_>| {
        b.group_forced(|b| {
            call(b, n, "f", &["a", "b"]);
            b.space();
            b.keyword(n.t("as"));
            b.space();
            b.token(n.t("alias"));
            b.token(n.t(","));
            b.line();
            b.token(n.t("c"));
        });
    };
    assert_eq!(
        print_at(source, Mode::Sql, 17, build),
        "f(a, b) AS alias,\nc\n"
    );
    assert_eq!(
        print_at(source, Mode::Sql, 16, build),
        "f(\n    a,\n    b\n) AS alias,\nc\n"
    );
}

#[test]
fn nested_groups_break_from_the_outside_in() {
    let source = "f(g(a, b), c)";
    let build = |b: &mut Builder<'_>, n: &mut Next<'_>| {
        b.token(n.t("f"));
        b.token(n.t("("));
        b.group(|b| {
            b.indent(|b| {
                b.soft_line();
                call(b, n, "g", &["a", "b"]);
                b.token(n.t(","));
                b.line();
                b.token(n.t("c"));
            });
            b.soft_line();
        });
        b.token(n.t(")"));
    };
    assert_eq!(print_at(source, Mode::Sql, 13, build), "f(g(a, b), c)\n");
    assert_eq!(
        print_at(source, Mode::Sql, 12, build),
        "f(\n    g(a, b),\n    c\n)\n"
    );
    assert_eq!(
        print_at(source, Mode::Sql, 11, build),
        "f(\n    g(\n        a,\n        b\n    ),\n    c\n)\n"
    );
}

#[test]
fn a_hard_line_breaks_every_group_around_it() {
    let out = print("a b c d", |b, n| {
        b.group(|b| {
            b.token(n.t("a"));
            b.line();
            b.group(|b| {
                b.token(n.t("b"));
                b.line();
                b.group(|b| {
                    b.token(n.t("c"));
                    b.hard_line();
                });
            });
        });
        b.group(|b| {
            b.token(n.t("d"));
        });
    });
    assert_eq!(out, "a\nb\nc\nd\n");
}

#[test]
fn a_forced_group_and_its_parents_never_print_flat() {
    let out = print("select a, b", |b, n| {
        b.group(|b| {
            b.group_forced(|b| {
                b.keyword(n.t("select"));
                b.indent(|b| {
                    b.line();
                    b.token(n.t("a"));
                    b.token(n.t(","));
                    b.line();
                    b.token(n.t("b"));
                });
            });
        });
    });
    assert_eq!(out, "SELECT\n    a,\n    b\n");
    let out = print("select a", |b, n| {
        b.group(|b| {
            b.keyword(n.t("select"));
            b.indent(|b| {
                b.line();
                b.token(n.t("a"));
            });
        });
    });
    assert_eq!(out, "SELECT a\n");
}

#[test]
fn scopes_set_the_indentation_of_their_lines() {
    let out = print("a b c d e f", |b, n| {
        b.token(n.t("a"));
        b.indent(|b| {
            b.hard_line();
            b.token(n.t("b"));
            b.indent(|b| {
                b.hard_line();
                b.token(n.t("c"));
                b.dedent(|b| {
                    b.hard_line();
                    b.token(n.t("d"));
                });
                b.at_root(|b| {
                    b.hard_line();
                    b.token(n.t("e"));
                });
            });
        });
        // A line break is indented as the scope it is in.
        b.indent(Builder::hard_line);
        b.token(n.t("f"));
    });
    assert_eq!(out, "a\n    b\n        c\n    d\ne\n    f\n");
}

#[test]
fn after_a_comment_the_next_token_starts_a_line_in_its_own_scope() {
    let out = print("group by a -- c\n;", |b, n| {
        b.keyword(n.t("group"));
        b.space();
        b.keyword(n.t("by"));
        b.group(|b| {
            b.indent(|b| {
                b.line();
                b.token(n.t("a"));
                b.trailing_comment(n.t("-- c"));
            });
        });
        b.token(n.t(";"));
    });
    assert_eq!(out, "GROUP BY\n    a  -- c\n;\n");
}

#[test]
fn hugged_brackets_close_at_the_indentation_of_their_line() {
    let source = "where a = 1 and b in (select 1)";
    let out = print(source, |b, n| {
        b.keyword(n.t("where"));
        b.space();
        b.group_forced(|b| {
            b.token(n.t("a"));
            b.space();
            b.token(n.t("="));
            b.space();
            b.token(n.t("1"));
            b.indent(|b| {
                b.line();
                b.keyword(n.t("and"));
                b.space();
                b.token(n.t("b"));
                b.space();
                b.keyword(n.t("in"));
                b.space();
                b.token(n.t("("));
                b.hug(|b| {
                    b.indent(|b| {
                        b.hard_line();
                        b.keyword(n.t("select"));
                        b.space();
                        b.token(n.t("1"));
                    });
                    b.hard_line();
                    b.token(n.t(")"));
                });
            });
        });
    });
    assert_eq!(
        out,
        "WHERE a = 1\n    AND b IN (\n        SELECT 1\n    )\n"
    );
}

/// `GROUP BY keys WITH TOTALS`: on the line of the keys when they fit,
/// else on a line of its own at the clause's indentation.
fn group_by(width: usize) -> String {
    let source = "group by hour, site with totals";
    print_at(source, Mode::Sql, width, |b, n| {
        b.group_forced(|b| {
            b.keyword(n.t("group"));
            b.space();
            b.keyword(n.t("by"));
            let keys = b.group(|b| {
                b.indent(|b| {
                    b.line();
                    b.token(n.t("hour"));
                    b.token(n.t(","));
                    b.line();
                    b.token(n.t("site"));
                });
            });
            b.if_break(keys, Gap::Line, Gap::Space);
            b.keyword(n.t("with"));
            b.space();
            b.keyword(n.t("totals"));
        });
    })
}

#[test]
fn if_break_follows_the_group_it_names() {
    assert_eq!(group_by(100), "GROUP BY hour, site WITH TOTALS\n");
    assert_eq!(group_by(25), "GROUP BY\n    hour,\n    site\nWITH TOTALS\n");
}

#[test]
fn blank_lines_are_kept_only_where_a_line_breaks() {
    let build = |b: &mut Builder<'_>, n: &mut Next<'_>| {
        b.group(|b| {
            b.token(n.t("a"));
            b.line();
            b.blank_lines(1);
            b.token(n.t("b"));
        });
    };
    assert_eq!(print_at("a\n\nb", Mode::Sql, 100, build), "a b\n");
    assert_eq!(print_at("a\n\nb", Mode::Sql, 2, build), "a\n\nb\n");
}

#[test]
fn blank_lines_collapse_to_two_and_never_follow_open_or_precede_close() {
    let out = print("a ( b ) c", |b, n| {
        b.blank_lines(2);
        b.token(n.t("a"));
        b.hard_line();
        b.blank_lines(5);
        b.token(n.t("("));
        b.hard_line();
        b.blank_lines(1);
        b.token(n.t("b"));
        b.hard_line();
        b.blank_lines(1);
        b.token(n.t(")"));
        b.blank_line();
        b.token(n.t("c"));
    });
    assert_eq!(out, "a\n\n\n(\nb\n)\n\nc\n");
}

/// Statements separated as the style asks: the input's blank lines (here
/// none), and one before and after a statement on several lines.
#[test]
fn a_blank_line_can_wait_for_a_group_built_after_it() {
    let source = "select 1; select a, b; select 2;";
    let out = print(source, |b, n| {
        let mut previous = None;
        for items in [&["1"][..], &["a", "b"], &["2"]] {
            let id = b.reserve_group();
            if let Some(previous) = previous {
                b.hard_line();
                b.blank_line_if_broken(previous);
                b.blank_line_if_broken(id);
            }
            b.group_reserved(id, |b| {
                b.keyword(n.t("select"));
                b.group(|b| {
                    if items.len() > 1 {
                        b.force_break();
                    }
                    b.indent(|b| {
                        for (k, item) in items.iter().enumerate() {
                            if k > 0 {
                                b.token(n.t(","));
                            }
                            b.line();
                            b.token(n.t(item));
                        }
                    });
                });
            });
            b.token(n.t(";"));
            previous = Some(id);
        }
    });
    assert_eq!(out, "SELECT 1;\n\nSELECT\n    a,\n    b;\n\nSELECT 2;\n");
}

#[test]
fn frozen_whitespace_is_printed_byte_for_byte() {
    let out = print("a  \t b\n\n  c d", |b, n| {
        b.token(n.t("a"));
        b.frozen();
        b.hard_line();
        b.token(n.t("b"));
        b.space();
        b.frozen();
        b.token(n.t("c"));
        b.indent(|b| {
            b.hard_line();
            b.token(n.t("d"));
        });
    });
    assert_eq!(out, "a  \t b\n\n  c\n    d\n");
}

#[test]
fn frozen_tokens_keep_the_boundaries_between_them() {
    // Frozen: the space after `a`, and `(`, `b` and `c`; not `)`.
    let out = print("a (b)  c", |b, n| {
        b.freeze(1);
        b.freeze(2);
        b.freeze(3);
        b.freeze(6);
        b.token(n.t("a"));
        b.hard_line();
        b.token(n.t("("));
        b.space();
        b.token(n.t("b"));
        b.space();
        b.token(n.t(")"));
        b.space();
        b.token(n.t("c"));
    });
    assert_eq!(out, "a (b ) c\n");
}

#[test]
fn whitespace_the_renderer_keeps_is_frozen_in_templates() {
    let source = "a\u{feff} b";
    let out = print_at(source, Mode::Jinja, 100, |b, n| {
        b.token(n.t("a"));
        b.hard_line();
        b.token(n.t("b"));
    });
    assert_eq!(out, "a\u{feff} b\n");
    let out = print_at(source, Mode::Sql, 100, |b, n| {
        b.token(n.t("a"));
        b.hard_line();
        b.token(n.t("b"));
    });
    assert_eq!(out, "a\nb\n");
}

#[test]
fn verbatim_text_is_reindented_as_a_block() {
    let source = "x\n  sum(v) over (partition by k\n                 order by ts) as s";
    let out = print(source, |b, n| {
        b.token(n.t("x"));
        b.indent(|b| {
            b.hard_line();
            b.token(n.t("sum"));
            b.token(n.t("("));
            b.token(n.t("v"));
            b.token(n.t(")"));
            b.space();
            b.keyword(n.t("over"));
            b.space();
            let first = n.t("(");
            for t in ["partition", "by", "k", "order", "by", "ts"] {
                n.t(t);
            }
            b.verbatim(first, n.t(")"));
            b.space();
            b.keyword(n.t("as"));
            b.space();
            b.token(n.t("s"));
        });
    });
    assert_eq!(
        out,
        "x\n    sum(v) OVER (partition by k\n                   order by ts) AS s\n"
    );
}

#[test]
fn verbatim_text_keeps_the_bytes_of_tokens_that_span_lines() {
    let source =
        "  alter table t\n      add column x String default 'a\n  b',\n\n\n      drop column y;";
    let out = print(source, |b, n| {
        let first = n.t("alter");
        for t in [
            "table", "t", "add", "column", "x", "String", "default", "'a\n  b'", ",",
        ] {
            n.t(t);
        }
        for t in ["drop", "column"] {
            n.t(t);
        }
        b.verbatim(first, n.t("y"));
        b.token(n.t(";"));
    });
    assert_eq!(
        out,
        "alter table t\n    add column x String default 'a\n  b',\n\n    drop column y;\n"
    );
}

#[test]
fn verbatim_blank_lines_never_follow_open_or_precede_close() {
    let source = "f(\n\n  a,  \n\n\n  b\n\n)";
    let out = print(source, |b, n| {
        let first = n.t("f");
        for t in ["(", "a", ",", "b"] {
            n.t(t);
        }
        b.verbatim(first, n.t(")"));
    });
    assert_eq!(out, "f(\n  a,\n\n  b\n)\n");
}

#[test]
fn exact_verbatim_text_keeps_every_byte() {
    let source = "x [1,  2,\n 3]::t";
    let out = print(source, |b, n| {
        b.token(n.t("x"));
        b.indent(|b| {
            b.hard_line();
            let first = n.t("[");
            for t in ["1", ",", "2", ","] {
                n.t(t);
            }
            n.t("3");
            b.verbatim_exact(first, n.t("]"));
            b.token(n.t("::"));
            b.token(n.t("t"));
        });
    });
    assert_eq!(out, "x\n    [1,  2,\n 3]::t\n");
}

#[test]
fn a_block_of_comments_keeps_its_relative_indentation() {
    let source = "a\n      -- one\n      --   two\n\n\n        -- three\n  b";
    let out = print(source, |b, n| {
        b.token(n.t("a"));
        b.indent(|b| {
            let first = n.t("-- one");
            n.t("--   two");
            b.comment_block(first, n.t("-- three"));
            b.token(n.t("b"));
        });
    });
    assert_eq!(
        out,
        "a\n    -- one\n    --   two\n\n      -- three\n    b\n"
    );
}

#[test]
fn a_trailing_comment_stays_two_spaces_after_its_line() {
    let source = "select a, -- first\n b";
    let out = print(source, |b, n| {
        b.group(|b| {
            b.keyword(n.t("select"));
            b.indent(|b| {
                b.line();
                b.token(n.t("a"));
                b.token(n.t(","));
                // The layout's line break before the comment is dropped.
                b.line();
                b.trailing_comment(n.t("-- first"));
                b.token(n.t("b"));
            });
        });
    });
    assert_eq!(out, "SELECT\n    a,  -- first\n    b\n");
}

#[test]
fn a_comment_pins_a_comma_where_the_author_put_it() {
    let source = "select a -- slug\n, b";
    let out = print(source, |b, n| {
        b.group_forced(|b| {
            b.keyword(n.t("select"));
            b.indent(|b| {
                b.line();
                b.token(n.t("a"));
                b.trailing_comment(n.t("-- slug"));
                b.token(n.t(","));
                b.space();
                b.token(n.t("b"));
            });
        });
    });
    assert_eq!(out, "SELECT\n    a  -- slug\n    , b\n");
}

#[test]
fn a_semicolon_after_a_trailing_comment_goes_on_its_own_line() {
    let source = "select 1 -- one\n;";
    let out = print(source, |b, n| {
        b.keyword(n.t("select"));
        b.space();
        b.token(n.t("1"));
        b.trailing_comment(n.t("-- one"));
        b.token(n.t(";"));
    });
    assert_eq!(out, "SELECT 1  -- one\n;\n");
}

#[test]
fn a_trailing_comment_without_code_before_it_stands_alone() {
    let source = "-- a\n-- b\nc";
    let out = print(source, |b, n| {
        b.trailing_comment(n.t("-- a"));
        b.trailing_comment(n.t("-- b"));
        b.token(n.t("c"));
    });
    assert_eq!(out, "-- a\n-- b\nc\n");
}

/// Printed once, a trailing comment that cannot trail is an own-line
/// comment; formatted again it is built as one, so it must print alike.
#[test]
fn a_comment_that_cannot_trail_prints_as_an_own_line_comment() {
    let source = "where\n-- c\nx";
    let place = |trailing: bool| {
        print(source, |b, n| {
            b.block_tag(n.t("where"));
            b.indent(|b| {
                let comment = n.t("-- c");
                if trailing {
                    b.trailing_comment(comment);
                } else {
                    b.own_line_comment(comment);
                }
                b.token(n.t("x"));
            });
        })
    };
    assert_eq!(place(true), "where\n    -- c\n    x\n");
    assert_eq!(place(false), place(true));
}

#[test]
fn an_own_line_comment_takes_the_indentation_of_its_scope() {
    let source = "where a /* one\n  two */ -- c\n and b";
    let out = print(source, |b, n| {
        b.keyword(n.t("where"));
        b.space();
        b.group(|b| {
            b.token(n.t("a"));
            b.indent(|b| {
                b.line();
                b.own_line_comment(n.t("/* one\n  two */"));
                b.own_line_comment(n.t("-- c"));
                b.keyword(n.t("and"));
                b.space();
                b.token(n.t("b"));
            });
        });
    });
    assert_eq!(out, "WHERE a\n    /* one\n  two */\n    -- c\n    AND b\n");
}

#[test]
fn an_inline_block_comment_is_an_atom() {
    let source = "f(a /* x */, b)";
    let build = |b: &mut Builder<'_>, n: &mut Next<'_>| {
        b.token(n.t("f"));
        b.token(n.t("("));
        b.group(|b| {
            b.indent(|b| {
                b.soft_line();
                b.token(n.t("a"));
                b.space();
                b.token(n.t("/* x */"));
                b.token(n.t(","));
                b.line();
                b.token(n.t("b"));
            });
            b.soft_line();
        });
        b.token(n.t(")"));
    };
    assert_eq!(print(source, build), "f(a /* x */, b)\n");
}

#[test]
fn block_tags_go_on_lines_of_their_own_and_closers_align() {
    let source = "where true\n and a\n {% if x -%}\n and b\n {% endif -%}\n group by c";
    let out = print_at(source, Mode::Jinja, 100, |b, n| {
        b.keyword(n.t("where"));
        b.space();
        let mut opener = None;
        b.group(|b| {
            b.literal_word(n.t("true"));
            b.indent(|b| {
                b.line();
                b.keyword(n.t("and"));
                b.space();
                b.token(n.t("a"));
                opener = Some(b.block_tag(n.t("{% if x -%}")));
                b.keyword(n.t("and"));
                b.space();
                b.token(n.t("b"));
            });
        });
        // The closer is outside the indented operands, as it is in the
        // tree, and still aligns with its opener.
        b.block_tag_aligned(n.t("{% endif -%}"), opener.expect("opener"));
        b.keyword(n.t("group"));
        b.space();
        b.keyword(n.t("by"));
        b.space();
        b.token(n.t("c"));
    });
    assert_eq!(
        out,
        "WHERE true\n    AND a\n    {% if x -%}\n    AND b\n    {% endif -%}\nGROUP BY c\n"
    );
}

#[test]
fn jinja_expressions_keep_their_spacing() {
    // `-{{ x }}` would render `- -1` as a comment if `x` is `-1`.
    let source = "a -{{ x }}({{ y }}) , {{ z }}";
    let out = print_at(source, Mode::Jinja, 100, |b, n| {
        b.token(n.t("a"));
        b.space();
        b.token(n.t("-"));
        b.space();
        b.token(n.t("{{ x }}"));
        b.line();
        b.token(n.t("("));
        b.token(n.t("{{ y }}"));
        b.token(n.t(")"));
        b.token(n.t(","));
        b.token(n.t("{{ z }}"));
    });
    assert_eq!(out, "a -{{ x }}({{ y }}), {{ z }}\n");
}

#[test]
fn tokens_that_would_lex_as_others_are_kept_apart() {
    // Each pair, printed with nothing between, would lex as other tokens:
    // `--1` is a comment, `//* c */` a comment, `x'1'` a string, `$$$$` a
    // heredoc opener, `1e+5` one number, `a.5` a word and a number.
    for source in [
        "- -1",
        "/ /* c */",
        "x '1'",
        "$$ $$",
        "1e +5",
        "a b",
        "1 .5",
        "/ *",
    ] {
        let out = print(source, |b, n| all_tokens(b, n, false));
        assert_eq!(out.trim_end(), source, "{source:?}");
    }
    let out = print("(a).b, f(x)::t", |b, n| all_tokens(b, n, false));
    assert_eq!(out, "(a).b,f(x)::t\n");
}

/// Adds every token, with a space before each if `spaced`.
fn all_tokens(b: &mut Builder<'_>, n: &mut Next<'_>, spaced: bool) {
    let (tokens, source) = (n.tokens, n.source);
    for token in tokens {
        if token.kind() != TokenKind::Whitespace {
            if spaced {
                b.space();
            }
            b.token(n.t(token.text(source)));
        }
    }
}

#[test]
fn line_comments_lose_the_whitespace_at_their_end() {
    let out = print("-- c \r\n#  \nx", |b, n| {
        b.own_line_comment(n.t("-- c \r"));
        b.own_line_comment(n.t("#  "));
        b.token(n.t("x"));
    });
    assert_eq!(out, "-- c\n#  \nx\n");
}

#[test]
fn the_output_ends_in_one_newline_unless_that_changes_the_last_token() {
    let one = |source: &str| print(source, |b, n| all_tokens(b, n, true));
    assert_eq!(one("select 1\n\n\n"), "select 1\n");
    assert_eq!(one("\n\nselect 1"), "select 1\n");
    // `1e` at the very end is a bare word, with a newline after it a number.
    assert_eq!(one("select 1e"), "select 1e");
    assert_eq!(one(""), "");
    let source = "a b\n\n  ";
    let out = print(source, |b, n| {
        b.set_ending(Ending::AsInput);
        b.token(n.t("a"));
        b.space();
        b.token(n.t("b"));
    });
    assert_eq!(out, "a b\n\n  ");
}

#[test]
fn a_byte_order_mark_stays_and_a_leading_dot_keeps_a_space() {
    let out = print("\u{feff}\n\nselect", |b, n| b.keyword(n.t("select")));
    assert_eq!(out, "\u{feff}SELECT\n");
    let out = print(" .x", |b, n| {
        b.token(n.t("."));
        b.token(n.t("x"));
    });
    assert_eq!(out, " .x\n");
    // At the very start, `.` is a number; indented, it would be a `Dot`.
    let out = print(". a", |b, n| {
        b.indent(|b| {
            b.token(n.t("."));
            b.line();
            b.token(n.t("a"));
        });
    });
    assert_eq!(out, ".\n    a\n");
}

#[test]
fn a_token_that_spans_lines_breaks_its_groups() {
    let source = "f('a\nb', c)";
    let out = print(source, |b, n| call(b, n, "f", &["'a\nb'", "c"]));
    assert_eq!(out, "f(\n    'a\nb',\n    c\n)\n");
}

#[test]
fn the_token_check_names_what_is_missing_or_repeated() {
    let source = "select a -- c\nfrom t";
    let tokens = crate::lexer::tokenize(source, Mode::Sql);
    let check = |build: &dyn Fn(&mut Builder<'_>)| {
        let mut b = Builder::new(source, &tokens, Mode::Sql);
        build(&mut b);
        b.finish().check_tokens().map_err(|e| e.to_string())
    };
    assert_eq!(
        check(&|b| (0..tokens.len()).step_by(2).for_each(|i| b.token(i))),
        Ok(())
    );
    let missing = check(&|b| {
        for i in [0, 2, 6, 8] {
            b.token(i);
        }
    })
    .expect_err("the comment is missing");
    assert!(
        missing.contains("token 4 (LineComment \"-- c\") is not printed"),
        "{missing}"
    );
    let repeated = check(&|b| {
        for i in [0, 2, 2, 4, 6, 8] {
            b.token(i);
        }
    })
    .expect_err("a token is repeated");
    assert!(
        repeated.contains("token 2 (BareWord \"a\") is printed again"),
        "{repeated}"
    );
    let short = check(&|b| b.token(0)).expect_err("the rest is missing");
    assert!(
        short.contains("token 2 (BareWord \"a\") and every token after it"),
        "{short}"
    );
    let verbatim = check(&|b| {
        b.token(0);
        b.verbatim(2, 6);
        b.token(8);
    });
    assert_eq!(verbatim, Ok(()));
}

#[test]
fn deep_nesting_prints_without_recursion() {
    const DEPTH: usize = 100_000;
    let source = "x ".repeat(DEPTH);
    let tokens = crate::lexer::tokenize(&source, Mode::Sql);
    let mut b = Builder::new(&source, &tokens, Mode::Sql);
    let mut open = Vec::new();
    for i in 0..DEPTH {
        open.push(b.open_group());
        // Indentation that grows with depth would make the output itself
        // quadratic; a hug keeps it at the line's.
        open.push(if i % 2 == 0 {
            b.open_indent()
        } else {
            b.open_dedent()
        });
        open.push(b.open_hug());
        b.token(2 * i);
        b.line();
    }
    while let Some(scope) = open.pop() {
        b.close(scope);
    }
    let doc = b.finish();
    doc.check_tokens().expect("tokens");
    let out = doc.print(&PrintOptions::default());
    // The innermost groups, about fifty tokens, fit on one line.
    let lines = out.lines().count();
    assert!((DEPTH - 60..DEPTH).contains(&lines), "{lines} lines");
    let wide = doc.print(&PrintOptions::default().with_line_width(usize::MAX));
    assert_eq!(wide.lines().count(), 1);
}

#[test]
fn a_node_takes_sixteen_bytes() {
    assert_eq!(std::mem::size_of::<ir::Node>(), 16);
}

#[test]
fn a_document_dumps_as_indented_nodes() {
    let source = "select a -- c\n, b";
    let tokens = crate::lexer::tokenize(source, Mode::Sql);
    let mut b = Builder::new(source, &tokens, Mode::Sql);
    b.group_forced(|b| {
        b.keyword(0);
        b.indent(|b| {
            b.line();
            b.token(2);
            b.trailing_comment(4);
            b.token(6);
            b.space();
            b.token(8);
        });
    });
    insta::assert_snapshot!(format!("{:?}", b.finish()), @r#"
    group #0 forced
      0 "select" as "SELECT"
      indent
        line
        2 "a"
        4 "-- c" trailing
        6 ","
        space
        8 "b"
    "#);
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// The fast measurement decides every group as printing it does.
    #[test]
    fn the_fast_measurement_agrees_with_printing(
        data in prop::collection::vec(any::<u8>(), 0..600),
        width in 0usize..60,
        indent in 0usize..6,
        jinja in any::<bool>(),
    ) {
        let mode = if jinja { Mode::Jinja } else { Mode::Sql };
        let mut bytes = doc_gen::Bytes::new(&data);
        let n = 1 + bytes.below(60);
        let source = doc_gen::source(&mut bytes, mode, n);
        let tokens = doc_gen::tokens(&source, mode);
        let doc = doc_gen::document(&mut bytes, &source, &tokens, mode, doc_gen::Shape::ALL);
        let options = PrintOptions::default().with_line_width(width).with_indent_width(indent);
        prop_assert_eq!(doc.print(&options), print::print_reference(&doc, &options));
    }
}
