//! Documents built by hand for the examples of the layout proposal
//! (`docs/spikes/2026-11-10-layout-proposal.md`), printed and compared with
//! the proposal's text byte for byte: the proof that the IR can say what
//! the style asks for.
//!
//! Each example is printed from two inputs with the same tokens: the
//! expected text flattened (line breaks become spaces, except next to
//! comments and tags and where a blank line is, and keywords go to lower
//! case), and the expected text itself. Both must print the expected text,
//! and every output must pass the verifier.

use chairlift::doc::Builder;
use chairlift::doc::GroupId;
use chairlift::doc::PrintOptions;
use chairlift::doc::TagId;
use chairlift::lexer::Mode;
use chairlift::lexer::Token;
use chairlift::lexer::TokenKind;
use chairlift::lexer::tokenize;
use chairlift::split::split;
use chairlift::verify::Facts;
use chairlift::verify::verify;

#[path = "support/proposal.rs"]
mod proposal;

/// A layout in progress: a builder and the next token of the source.
struct L<'s> {
    b: Builder<'s>,
    tokens: &'s [Token],
    source: &'s str,
    mode: Mode,
    at: usize,
}

/// A token of a fragment of expected text.
#[derive(Clone, Copy)]
struct Item<'t> {
    kind: TokenKind,
    text: &'t str,
    spaced: bool,
}

const LOWER_LITERALS: [&str; 5] = ["true", "false", "inf", "infinity", "nan"];

impl L<'_> {
    /// The index of the next token other than whitespace.
    fn peek(&self) -> usize {
        let mut i = self.at;
        while self.tokens[i].kind() == TokenKind::Whitespace {
            i += 1;
        }
        i
    }

    fn peek_kind(&self) -> Option<TokenKind> {
        let mut i = self.at;
        while i < self.tokens.len() && self.tokens[i].kind() == TokenKind::Whitespace {
            i += 1;
        }
        self.tokens.get(i).map(|t| t.kind())
    }

    /// Takes the next token, which must read `expected` in some case.
    fn take(&mut self, expected: &str) -> usize {
        let i = self.peek();
        let text = self.tokens[i].text(self.source);
        assert!(
            text.eq_ignore_ascii_case(expected),
            "token {i} is {text:?}, the layout expects {expected:?}"
        );
        self.at = i + 1;
        i
    }

    /// Prints the next token as `text`: re-cased if it is written in
    /// another case, as written otherwise.
    fn t(&mut self, text: &str) {
        let i = self.take(text);
        if self.tokens[i].text(self.source) == text {
            self.b.token(i);
        } else if LOWER_LITERALS.contains(&text) {
            self.b.literal_word(i);
        } else {
            self.b.keyword(i);
        }
    }

    /// Prints space-separated words: `GROUP BY`.
    fn kw(&mut self, words: &str) {
        for (k, word) in words.split(' ').enumerate() {
            if k > 0 {
                self.b.space();
            }
            self.t(word);
        }
    }

    /// Prints the next token, a comment: trailing if it trails its line in
    /// the input, on its own line otherwise.
    fn comment(&mut self) {
        let i = self.peek();
        assert!(
            self.tokens[i].kind().is_comment(),
            "token {i} is not a comment"
        );
        self.at = i + 1;
        if self.b.newlines_before(i) == 0 {
            self.b.trailing_comment(i);
        } else {
            self.b.own_line_comment(i);
        }
    }

    /// Prints the next token, a block tag.
    fn tag(&mut self) -> TagId {
        let i = self.peek();
        assert_eq!(self.tokens[i].kind(), TokenKind::JinjaBlock);
        self.at = i + 1;
        self.b.block_tag(i)
    }

    /// Prints the next token, a block tag that closes `opener`.
    fn closer(&mut self, opener: TagId) {
        let i = self.peek();
        assert_eq!(self.tokens[i].kind(), TokenKind::JinjaBlock);
        self.at = i + 1;
        self.b.block_tag_aligned(i, opener);
    }

    /// Prints comments up next, if any.
    fn comments(&mut self) {
        while self.peek_kind().is_some_and(TokenKind::is_comment) {
            self.comment();
        }
    }

    /// Prints a comment up next that trails its line, if any.
    fn trailing(&mut self) {
        if self.peek_kind().is_some_and(TokenKind::is_comment)
            && self.b.newlines_before(self.peek()) == 0
        {
            self.comment();
        }
    }

    /// The blank line the input has before the next token, if any.
    fn blank(&mut self) {
        let n = self.b.newlines_before(self.peek()).saturating_sub(1).min(1);
        self.b.blank_lines(u8::try_from(n).expect("small"));
    }

    fn sp(&mut self) {
        self.b.space();
    }

    fn line(&mut self) {
        self.b.line();
    }

    fn soft(&mut self) {
        self.b.soft_line();
    }

    fn hard(&mut self) {
        self.b.hard_line();
    }

    fn scope(&mut self, open: chairlift::doc::Open, f: impl FnOnce(&mut Self)) -> Option<GroupId> {
        let id = open.group();
        f(self);
        self.b.close(open);
        id
    }

    fn group(&mut self, f: impl FnOnce(&mut Self)) -> GroupId {
        let open = self.b.open_group();
        self.scope(open, f).expect("a group")
    }

    fn forced(&mut self, f: impl FnOnce(&mut Self)) -> GroupId {
        let open = self.b.open_group();
        self.b.force_break();
        self.scope(open, f).expect("a group")
    }

    fn indent(&mut self, f: impl FnOnce(&mut Self)) {
        let open = self.b.open_indent();
        self.scope(open, f);
    }

    fn hug(&mut self, f: impl FnOnce(&mut Self)) {
        let open = self.b.open_hug();
        self.scope(open, f);
    }

    /// Lays out a fragment written flat in `text`: tokens spaced as there,
    /// and the contents of every bracket a list that stays on one line if
    /// it fits and goes one item per line otherwise.
    fn inline(&mut self, text: &str) {
        let tokens = tokenize(text, self.mode);
        let mut items = Vec::new();
        let mut spaced = false;
        for token in &tokens {
            if token.kind() == TokenKind::Whitespace {
                spaced = true;
            } else {
                items.push(Item {
                    kind: token.kind(),
                    text: token.text(text),
                    spaced,
                });
                spaced = false;
            }
        }
        self.items(&items);
    }

    fn items(&mut self, items: &[Item<'_>]) {
        let mut i = 0;
        while i < items.len() {
            if i > 0 && items[i].spaced {
                self.sp();
            }
            self.t(items[i].text);
            if opens(items[i].kind) {
                let close = closing(items, i);
                self.list(&items[i + 1..close]);
                self.t(items[close].text);
                i = close;
            }
            i += 1;
        }
    }

    /// A bracket's contents: on one line if they fit, else one item per
    /// line, the closer at the indentation of the opener's line.
    fn list(&mut self, inner: &[Item<'_>]) {
        if inner.is_empty() {
            return;
        }
        let parts = split_commas(inner);
        self.group(|l| {
            l.indent(|l| {
                l.soft();
                for (k, part) in parts.iter().enumerate() {
                    if k > 0 {
                        l.t(",");
                        l.trailing();
                        l.line();
                        l.comments();
                    }
                    l.items(part);
                }
            });
            l.soft();
        });
    }

    /// `( query )` hugging its brackets.
    fn hugged(&mut self, query: impl FnOnce(&mut Self)) {
        self.t("(");
        self.hug(|l| {
            l.indent(|l| {
                l.hard();
                query(l);
            });
            l.hard();
            l.t(")");
        });
    }

    /// A `SELECT` list: always one item per line with more than one item.
    fn select(&mut self, items: &[&str]) {
        let layout = |l: &mut Self| {
            l.t("SELECT");
            l.indent(|l| {
                for (k, item) in items.iter().enumerate() {
                    if k > 0 {
                        l.t(",");
                        l.trailing();
                    }
                    l.line();
                    l.blank();
                    l.comments();
                    l.inline(item);
                }
            });
        };
        if items.len() > 1 {
            self.forced(layout);
        } else {
            self.group(layout);
        }
    }

    /// A clause on its own line: `KEYWORD body`.
    fn clause(&mut self, keyword: &str, body: &str) {
        self.hard();
        self.kw(keyword);
        self.sp();
        self.inline(body);
    }

    /// A clause whose list stays on its line if it fits, else one item per
    /// line, indented: `GROUP BY`, `ORDER BY`, `SETTINGS`.
    fn list_clause(&mut self, keyword: &str, items: &[&str]) -> GroupId {
        self.hard();
        self.kw(keyword);
        self.group(|l| {
            l.indent(|l| {
                for (k, item) in items.iter().enumerate() {
                    if k > 0 {
                        l.t(",");
                        l.trailing();
                    }
                    l.line();
                    l.blank();
                    l.comments();
                    l.inline(item);
                }
            });
        })
    }

    /// Conditions in the hanging style: the first on the keyword's line,
    /// each `AND`/`OR` operand after it on a line of its own, one level in,
    /// unless they all fit on the line.
    fn conditions(&mut self, keyword: &str, first: &str, rest: &[&str]) {
        self.hard();
        self.kw(keyword);
        self.sp();
        self.group(|l| {
            l.inline(first);
            l.indent(|l| {
                for operand in rest {
                    l.line();
                    l.comments();
                    l.inline(operand);
                    l.trailing();
                }
            });
        });
    }

    /// Statements one after the other: the blank lines between them as in
    /// the input, up to two, and one before and after any statement that
    /// spans lines.
    fn statements(&mut self, n: usize, mut statement: impl FnMut(&mut Self, usize)) {
        let mut previous = None;
        for k in 0..n {
            let id = self.b.reserve_group();
            if let Some(previous) = previous {
                self.hard();
                let blank = self.b.newlines_before(self.peek()).saturating_sub(1).min(2);
                self.b.blank_lines(u8::try_from(blank).expect("small"));
                self.b.blank_line_if_broken(previous);
                self.b.blank_line_if_broken(id);
            }
            let open = self.b.open_reserved_group(id);
            statement(self, k);
            self.b.close(open);
            previous = Some(id);
        }
    }
}

fn opens(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::OpeningRoundBracket | TokenKind::OpeningSquareBracket
    )
}

fn closes(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::ClosingRoundBracket | TokenKind::ClosingSquareBracket
    )
}

fn closing(items: &[Item<'_>], open: usize) -> usize {
    let mut depth = 0;
    for (i, item) in items.iter().enumerate().skip(open) {
        if opens(item.kind) {
            depth += 1;
        } else if closes(item.kind) {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
    }
    panic!("unbalanced brackets");
}

fn split_commas<'a, 't>(items: &'a [Item<'t>]) -> Vec<&'a [Item<'t>]> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, item) in items.iter().enumerate() {
        if opens(item.kind) {
            depth += 1;
        } else if closes(item.kind) {
            depth -= 1;
        } else if item.kind == TokenKind::Comma && depth == 0 {
            parts.push(&items[start..i]);
            start = i + 1;
        }
    }
    parts.push(&items[start..]);
    parts
}

/// The expected text with its line breaks made spaces, except where a
/// blank line is and next to comments and block tags, and its upper-case
/// keywords in lower case.
fn flatten(expected: &str, mode: Mode) -> String {
    let tokens = tokenize(expected, mode);
    let breaks = |t: Option<&Token>| {
        t.is_some_and(|t| t.kind().is_comment() || t.kind() == TokenKind::JinjaBlock)
    };
    let mut out = String::new();
    for (i, token) in tokens.iter().enumerate() {
        let text = token.text(expected);
        match token.kind() {
            TokenKind::Whitespace => {
                let newlines = text.matches('\n').count();
                let near =
                    breaks(i.checked_sub(1).map(|j| &tokens[j])) || breaks(tokens.get(i + 1));
                if newlines >= 2 || (newlines == 1 && near) {
                    out.push_str(&"\n".repeat(newlines));
                } else {
                    out.push(' ');
                }
            }
            TokenKind::BareWord
                if token.keyword().is_some() && text.bytes().all(|b| !b.is_ascii_lowercase()) =>
            {
                out.push_str(&text.to_ascii_lowercase());
            }
            _ => out.push_str(text),
        }
    }
    out
}

/// Prints `source` with `layout` at `width`, checking the document's
/// tokens and verifying the output.
fn format_with(source: &str, mode: Mode, width: usize, layout: &dyn Fn(&mut L<'_>)) -> String {
    let split = split(source, mode);
    let tokens = split.tokens();
    let mut l = L {
        b: Builder::new(source, tokens, mode),
        tokens,
        source,
        mode,
        at: 0,
    };
    layout(&mut l);
    assert!(l.peek_kind().is_none(), "the layout leaves tokens out");
    let doc = l.b.finish();
    doc.check_tokens().expect("every token once");
    let out = doc.print(&PrintOptions::default().with_line_width(width));
    let mut facts = Facts::new();
    for token in doc.recased() {
        facts.recase(token);
    }
    if let Err(e) = verify(source, &out, mode, &facts) {
        panic!("{e}\n--- output:\n{out}");
    }
    out
}

/// Checks that `layout` prints `expected` at `width` from the flattened
/// text and from `expected` itself.
fn golden_at(expected: &str, mode: Mode, width: usize, layout: impl Fn(&mut L<'_>)) {
    let expected = expected.strip_prefix('\n').unwrap_or(expected);
    for source in [flatten(expected, mode), expected.to_owned()] {
        let out = format_with(&source, mode, width, &layout);
        assert_eq!(out, expected, "printed from:\n{source}");
    }
}

fn golden(expected: &str, layout: impl Fn(&mut L<'_>)) {
    golden_at(expected, Mode::Sql, 100, layout);
}

// Section 2b: lists.

#[test]
fn a_select_list_with_more_than_one_item_always_breaks() {
    golden(proposal::SELECT_LIST_SPLITS, |l| {
        l.select(&["database", "table", "name", "reason"]);
        l.clause("FROM", "system.detached_parts");
    });
}

#[test]
fn a_list_that_does_not_fit_goes_one_item_per_line() {
    golden(proposal::IN_LIST, |l| {
        l.kw("DELETE FROM");
        l.sp();
        l.inline("raw.events");
        l.clause(
            "WHERE",
            "event_type IN ('page_view', 'article_view', 'video_start', 'video_complete', \
                 'scroll_25', 'scroll_50', 'scroll_75', 'scroll_100', 'share', 'comment', \
                 'subscribe', 'login')",
        );
    });
}

#[test]
fn an_array_literal_goes_one_item_per_line_too() {
    golden(proposal::ARRAY_LITERAL, |l| {
        l.group(|l| {
            l.t("WITH");
            l.indent(|l| {
                l.line();
                l.inline(
                    "[1024, 4096, 16384, 65536, 262144, 1048576, 4194304, 16777216, \
                         67108864, 268435456, 1073741824] AS size_buckets_bytes",
                );
            });
        });
        l.hard();
        l.select(&["arrayMap(b -> formatReadableSize(b), size_buckets_bytes) AS labels"]);
    });
}

/// `CREATE TABLE name (columns)` with the column list's `(` alone on its
/// line, as `SHOW CREATE TABLE` prints it.
fn create_table(l: &mut L<'_>, name: &str, columns: &[&str]) {
    l.kw("CREATE TABLE");
    l.sp();
    l.inline(name);
    l.hard();
    l.t("(");
    l.indent(|l| {
        for (k, column) in columns.iter().enumerate() {
            if k > 0 {
                l.t(",");
                l.trailing();
            }
            l.hard();
            l.blank();
            l.comments();
            l.inline(column);
        }
        l.trailing();
    });
    l.hard();
    l.t(")");
}

#[test]
fn settings_go_one_per_line_when_they_do_not_fit() {
    golden(proposal::SETTINGS_LIST, |l| {
        create_table(l, "landing.events", &["event_time DateTime64(3, 'UTC')"]);
        l.clause("ENGINE", "= MergeTree");
        l.clause("ORDER BY", "event_time");
        l.list_clause(
            "SETTINGS",
            &[
                "ttl_only_drop_parts = 1",
                "merge_with_ttl_timeout = 900",
                "min_bytes_for_wide_part = 33554432",
                "index_granularity = 8192",
                "storage_policy = 'hot_cold'",
                "allow_nullable_key = 1",
            ],
        );
    });
}

#[test]
fn an_order_by_tuple_fits_or_hugs_one_key_per_line() {
    golden(proposal::ORDER_BY_FITS, |l| {
        create_table(
            l,
            "analytics.teaser_clicks",
            &[
                "site LowCardinality(String)",
                "sero_time DateTime('UTC')",
                "product_feature LowCardinality(String)",
                "content_id String",
            ],
        );
        l.clause("ENGINE", "= SummingMergeTree");
        l.clause("ORDER BY", "(site, sero_time, product_feature, content_id)");
    });
    golden(proposal::ORDER_BY_BREAKS, |l| {
        create_table(
            l,
            "analytics.teaser_clicks_by_variant",
            &[
                "site LowCardinality(String)",
                "sero_time DateTime('UTC')",
                "content_id String",
            ],
        );
        l.clause("ENGINE", "= SummingMergeTree");
        l.clause(
            "ORDER BY",
            "(site, toStartOfHour(sero_time), payload_product_feature, \
                 payload_data_data_teaser_id, payload_data_data_variant_id)",
        );
    });
}

// Section 2c: conditions and commas.

#[test]
fn conditions_hang_with_leading_operators() {
    golden(proposal::CONDITIONS_HANG, |l| {
        l.select(&["count()"]);
        l.clause("FROM", "raw.events");
        l.conditions(
            "WHERE",
            "event_date >= today() - 7",
            &[
                "AND event_type IN ('page_view', 'article_view')",
                "AND site != ''",
                "AND NOT is_bot",
            ],
        );
    });
}

#[test]
fn commas_trail_and_group_by_fits() {
    golden(proposal::TRAILING_COMMAS, |l| {
        l.select(&[
            "toStartOfHour(event_time) AS hour",
            "site",
            "count() AS views",
        ]);
        l.clause("FROM", "raw.events");
        l.list_clause("GROUP BY", &["hour", "site"]);
    });
}

// Section 2d: column definitions, not aligned.

#[test]
fn column_definitions_are_not_aligned() {
    golden(proposal::COLUMNS_NOT_ALIGNED, |l| {
        create_table(
            l,
            "analytics.sessions",
            &[
                "session_id UUID",
                "site LowCardinality(String) COMMENT 'Publication slug'",
                "started_at DateTime64(3, 'UTC') CODEC(Delta, ZSTD(1)) COMMENT 'First event of the session'",
                "duration_ms UInt32 CODEC(T64, ZSTD(1))",
                "is_logged_in Bool DEFAULT false",
            ],
        );
        l.clause("ENGINE", "= MergeTree");
        l.clause("ORDER BY", "(site, started_at)");
    });
}

// Section 2e: blank lines.

#[test]
fn blank_lines_between_statements_are_kept_up_to_two() {
    golden(proposal::BLANK_LINES_KEPT, |l| {
        l.comment();
        l.hard();
        l.blank();
        let statements = [
            "CREATE ROLE events_writer",
            "GRANT INSERT ON landing.events TO events_writer",
            "GRANT events_writer TO events_consumer",
            "ALTER USER events_consumer DEFAULT ROLE ALL",
        ];
        l.statements(statements.len(), |l, k| {
            l.inline(statements[k]);
            l.t(";");
        });
    });
}

#[test]
fn a_statement_on_several_lines_gets_a_blank_line_on_each_side() {
    golden(proposal::BLANK_AROUND_MULTILINE, |l| {
        l.statements(3, |l, k| {
            if k == 1 {
                l.select(&["a", "b"]);
                l.clause("FROM", "t");
            } else {
                l.select(&[["1", "", "2"][k]]);
            }
            l.t(";");
        });
    });
}

// Section 2f: comments.

#[test]
fn comments_in_a_column_list_and_between_clauses() {
    golden(proposal::COLUMN_COMMENTS, |l| {
        create_table(
            l,
            "analytics.page_views_1h",
            &[
                "hour DateTime('UTC')",
                "site LowCardinality(String)",
                "views UInt64",
            ],
        );
        l.hard();
        l.comments();
        l.clause("ENGINE", "= AggregatingMergeTree");
        l.clause("ORDER BY", "(site, hour)");
    });
}

#[test]
fn a_line_comment_breaks_the_group_it_is_in() {
    golden(proposal::LINE_COMMENT_BREAKS, |l| {
        l.select(&["count()"]);
        l.clause("FROM", "raw.events");
        l.clause("ORDER BY", "(site, event_time)");
    });
}

#[test]
fn a_comment_pins_a_leading_comma() {
    golden(proposal::PINNED_COMMA, |l| {
        l.forced(|l| {
            l.t("SELECT");
            l.indent(|l| {
                l.line();
                l.inline("toStartOfHour(event_time) AS hour");
                l.t(",");
                l.line();
                l.t("site");
                l.comment();
                // The comma after the comment starts the line, and the
                // item stays on it.
                l.t(",");
                l.sp();
                l.inline("count() AS views");
            });
        });
        l.clause("FROM", "raw.events");
        l.list_clause("GROUP BY", &["hour", "site"]);
    });
}

#[test]
fn comments_in_conditions() {
    golden(proposal::CONDITION_COMMENTS, |l| {
        l.select(&["count()"]);
        l.clause("FROM", "raw.events");
        l.conditions(
            "WHERE",
            "event_date >= today() - 7",
            &["AND NOT is_bot", "AND site != ''"],
        );
    });
}

#[test]
fn a_semicolon_after_a_trailing_comment_stands_alone() {
    golden(proposal::SEMICOLON_ALONE, |l| {
        l.select(&["1"]);
        l.comment();
        l.t(";");
    });
}

// Section 2g: Jinja.

#[test]
fn block_tags_stay_at_the_indentation_of_the_sql_they_wrap() {
    golden_at(proposal::BLOCK_TAGS, Mode::Jinja, 100, |l| {
        l.select(&["count()"]);
        l.clause("FROM", "raw.events");
        l.hard();
        l.t("WHERE");
        l.sp();
        let mut opener = None;
        l.group(|l| {
            l.t("true");
            l.indent(|l| {
                l.line();
                l.inline("AND event_type = 'page_view'");
                opener = Some(l.tag());
                l.inline("AND event_time >= {{ this.cutoff | sql_datetime }}");
            });
        });
        // The closer is after the conditions in the tree, at the level of
        // the clauses, and still lines up with its opener.
        l.closer(opener.expect("an opener"));
    });
}

#[test]
fn a_for_loop_stays_at_the_indentation_of_the_list() {
    golden_at(proposal::FOR_LOOP, Mode::Jinja, 100, |l| {
        l.forced(|l| {
            l.t("SELECT");
            l.indent(|l| {
                l.line();
                l.t("site");
                l.t(",");
                let opener = l.tag();
                l.inline("countIf(event_type = '{{ e }}') AS {{ e }}_count");
                l.t(",");
                l.closer(opener);
                l.inline("count() AS total");
            });
        });
        l.clause("FROM", "raw.events");
        l.list_clause("GROUP BY", &["site"]);
    });
}

// Section 2h: nesting.

#[test]
fn a_subquery_hugs_its_brackets() {
    golden(proposal::SUBQUERY_HUGS, |l| {
        l.select(&["site", "avg(views) AS avg_daily_views"]);
        l.hard();
        l.t("FROM");
        l.sp();
        l.hugged(|l| {
            l.select(&["site", "toDate(event_time) AS day", "count() AS views"]);
            l.clause("FROM", "raw.events");
            l.list_clause("GROUP BY", &["site", "day"]);
        });
        l.sp();
        l.kw("AS daily");
        l.list_clause("GROUP BY", &["site"]);
    });
}

#[test]
fn an_in_subquery_hugs_its_brackets() {
    golden(proposal::IN_SUBQUERY_HUGS, |l| {
        l.select(&["count()"]);
        l.clause("FROM", "raw.events");
        l.hard();
        l.t("WHERE");
        l.sp();
        l.inline("visitor_id IN");
        l.sp();
        l.hugged(|l| {
            l.select(&["visitor_id"]);
            l.clause("FROM", "raw.subscriptions");
            l.clause("WHERE", "status = 'active'");
        });
    });
}

#[test]
fn a_single_cte_hugs_with() {
    golden(proposal::SINGLE_CTE_HUGS, |l| {
        l.kw("WITH daily AS");
        l.sp();
        l.hugged(|l| {
            l.select(&["site", "toDate(event_time) AS day"]);
            l.clause("FROM", "raw.events");
        });
        l.hard();
        l.select(&["site", "count() AS days"]);
        l.clause("FROM", "daily");
        l.list_clause("GROUP BY", &["site"]);
    });
}

#[test]
fn a_materialized_view_ends_its_header_with_as() {
    golden(proposal::MATERIALIZED_VIEW, |l| {
        l.group(|l| {
            l.kw("CREATE MATERIALIZED VIEW");
            l.sp();
            l.inline("analytics.events_daily_mv");
            l.indent(|l| {
                l.line();
                l.t("TO");
                l.sp();
                l.inline("analytics.events_daily");
            });
            l.sp();
            l.t("AS");
        });
        l.hard();
        l.select(&["toDate(event_time) AS day", "event_name", "count() AS cnt"]);
        l.clause("FROM", "raw.events");
        l.list_clause("GROUP BY", &["day", "event_name"]);
    });
}

#[test]
fn case_expands_with_when_and_else_one_level_in() {
    golden(proposal::CASE_EXPANDS, |l| {
        l.forced(|l| {
            l.t("SELECT");
            l.indent(|l| {
                l.line();
                l.group(|l| {
                    l.t("CASE");
                    l.indent(|l| {
                        for (bound, label) in [
                            ("10000", "'bounce'"),
                            ("60000", "'skim'"),
                            ("300000", "'read'"),
                        ] {
                            l.line();
                            l.inline(&format!("WHEN duration_ms < {bound} THEN {label}"));
                        }
                        l.line();
                        l.inline("ELSE 'deep read'");
                    });
                    l.line();
                    l.t("END");
                });
                l.sp();
                l.inline("AS engagement");
                l.t(",");
                l.line();
                l.inline("count() AS sessions");
            });
        });
        l.clause("FROM", "analytics.sessions");
        l.list_clause("GROUP BY", &["engagement"]);
    });
}

/// `arrayFilter((url, ts) -> …, urls, timestamps)`: the lambda is one
/// argument, and its body breaks after `->`, one level in.
fn lambda(l: &mut L<'_>) {
    l.forced(|l| {
        l.t("SELECT");
        l.indent(|l| {
            l.line();
            l.t("arrayFilter");
            l.t("(");
            l.group(|l| {
                l.indent(|l| {
                    l.soft();
                    l.group(|l| {
                        l.inline("(url, ts) ->");
                        l.indent(|l| {
                            l.line();
                            l.inline(
                                "ts >= session_start AND NOT \
                                 startsWith(url, 'https://internal.example.com/')",
                            );
                        });
                    });
                    l.t(",");
                    l.line();
                    l.t("urls");
                    l.t(",");
                    l.line();
                    l.t("timestamps");
                });
                l.soft();
            });
            l.t(")");
            l.sp();
            l.inline("AS external_urls");
        });
    });
    l.clause("FROM", "analytics.sessions");
}

#[test]
fn a_lambda_stays_whole_when_it_fits() {
    golden(proposal::LAMBDA_WHOLE, lambda);
}

/// Not an example of the proposal: the same lambda at width 90, where the
/// rule says its body breaks after `->`.
#[test]
fn a_lambda_breaks_after_the_arrow() {
    golden_at(proposal::LAMBDA_BREAKS_90, Mode::Sql, 90, lambda);
}

#[test]
fn a_join_condition_breaks_before_on_with_leading_and() {
    golden(proposal::JOIN_ON, |l| {
        l.select(&["o.order_id", "p.price"]);
        l.clause("FROM", "raw.orders AS o");
        l.hard();
        l.group(|l| {
            l.inline("ASOF LEFT JOIN dim.prices AS p");
            l.indent(|l| {
                for condition in [
                    "ON p.sku = o.sku",
                    "AND p.currency = o.currency",
                    "AND o.ordered_at >= p.valid_from",
                ] {
                    l.line();
                    l.inline(condition);
                }
            });
        });
    });
}

#[test]
fn insert_select_keeps_the_columns_on_the_insert_line() {
    golden(proposal::INSERT_SELECT, |l| {
        l.inline("INSERT INTO analytics.events_daily (day, event_name, cnt)");
        l.hard();
        l.select(&["toDate(event_time) AS day", "event_name", "count() AS cnt"]);
        l.clause("FROM", "raw.events");
        l.clause("WHERE", "event_time < '2026-01-01 00:00:00'");
        l.list_clause("GROUP BY", &["day", "event_name"]);
    });
}

/// `HEAD item, item…;`: the items on the head's line if they fit, else
/// one per line, indented; `forced` with more than one.
fn command_list(l: &mut L<'_>, head: &str, items: &[&str]) {
    let layout = |l: &mut L<'_>| {
        l.inline(head);
        l.indent(|l| {
            for (k, item) in items.iter().enumerate() {
                if k > 0 {
                    l.t(",");
                }
                l.line();
                l.inline(item);
            }
        });
    };
    if items.len() > 1 {
        l.forced(layout);
    } else {
        l.group(layout);
    }
    l.t(";");
}

#[test]
fn alter_table_commands_are_a_list() {
    golden(proposal::ALTER_TABLE, |l| {
        l.statements(2, |l, k| {
            if k == 0 {
                command_list(
                    l,
                    "ALTER TABLE raw.events",
                    &["ADD COLUMN IF NOT EXISTS element String AFTER duration_ms"],
                );
            } else {
                command_list(
                    l,
                    "ALTER TABLE analytics.page_views_1h",
                    &[
                        "ADD COLUMN referrer LowCardinality(String) DEFAULT '' AFTER site",
                        "MODIFY TTL hour + INTERVAL 180 DAY",
                    ],
                );
            }
        });
    });
}

#[test]
fn grant_breaks_before_on_to_and_with_grant_option() {
    golden(proposal::GRANT, |l| {
        l.statements(2, |l, k| {
            l.group(|l| {
                if k == 0 {
                    l.kw("GRANT SELECT");
                    l.indent(|l| {
                        l.line();
                        l.inline("ON analytics.page_views_1h");
                        l.line();
                        l.inline("TO reader");
                    });
                } else {
                    l.inline("GRANT SELECT, INSERT, ALTER UPDATE, ALTER DELETE");
                    l.indent(|l| {
                        l.line();
                        l.inline("ON analytics.page_views_1h_with_a_long_name");
                        l.line();
                        l.inline("TO analytics_writer, backfill_job, maintenance");
                        l.line();
                        l.kw("WITH GRANT OPTION");
                    });
                }
            });
            l.t(";");
        });
    });
}

// Section 2a: line width.

/// The proposal's width example: the same document at 120, 100 and 80.
fn widths(l: &mut L<'_>) {
    l.forced(|l| {
        l.t("SELECT");
        l.indent(|l| {
            l.line();
            l.inline("toStartOfHour(event_time) AS hour");
            l.t(",");
            l.line();
            l.t("round");
            l.t("(");
            l.group(|l| {
                l.indent(|l| {
                    l.soft();
                    // `a * b / c` is `(a * b) / c`: the division breaks
                    // before its operator, one level in.
                    l.group(|l| {
                        l.inline("100 * countIf(event_type = 'subscribe')");
                        l.indent(|l| {
                            l.line();
                            l.inline("/ nullIf(countIf(event_type = 'page_view'), 0)");
                        });
                    });
                    l.t(",");
                    l.line();
                    l.t("2");
                });
                l.soft();
            });
            l.t(")");
            l.sp();
            l.inline("AS conversion_pct");
            l.t(",");
            l.line();
            l.inline(
                "arrayStringConcat(arraySlice(groupUniqArray(referrer_domain), 1, 5), ', ') \
                 AS top_referrer_domains",
            );
        });
    });
    l.clause("FROM", "raw.events");
    l.list_clause("GROUP BY", &["hour"]);
}

#[test]
fn one_document_at_three_widths() {
    golden_at(proposal::WIDTH_120, Mode::Sql, 120, widths);
    golden_at(proposal::WIDTH_100, Mode::Sql, 100, widths);
    golden_at(proposal::WIDTH_80, Mode::Sql, 80, widths);
}

// Section 3: ClickHouse-specific constructs.

#[test]
fn a_projection_hugs_and_an_index_stays_on_one_line() {
    golden(proposal::PROJECTION_AND_INDEX, |l| {
        l.kw("CREATE TABLE");
        l.sp();
        l.inline("analytics.page_views ON CLUSTER '{cluster}'");
        l.hard();
        l.t("(");
        l.indent(|l| {
            for column in [
                "event_time DateTime64(3, 'UTC') CODEC(Delta, ZSTD(1))",
                "visitor_id String COMMENT 'Anonymous, 16 hex digits'",
                "INDEX visitor_bf visitor_id TYPE bloom_filter(0.01) GRANULARITY 4",
            ] {
                l.hard();
                l.inline(column);
                l.t(",");
            }
            l.hard();
            l.inline("PROJECTION by_visitor");
            l.sp();
            l.hugged(|l| {
                l.select(&["*"]);
                l.clause("ORDER BY", "visitor_id");
            });
        });
        l.hard();
        l.t(")");
        l.clause("ENGINE", "= ReplicatedMergeTree");
        l.clause("ORDER BY", "tuple()");
        l.list_clause("SETTINGS", &["index_granularity = 8192"]);
        l.clause("COMMENT", "'One row per page view'");
    });
}

#[test]
fn storage_clauses_go_one_per_line_in_the_authors_order() {
    golden(proposal::STORAGE_CLAUSES, |l| {
        create_table(
            l,
            "analytics.visits",
            &[
                "site LowCardinality(String)",
                "event_time DateTime",
                "visitor_id String",
            ],
        );
        l.clause("ENGINE", "= MergeTree");
        l.clause("PRIMARY KEY", "(site, toStartOfHour(event_time))");
        l.clause(
            "ORDER BY",
            "(site, toStartOfHour(event_time), cityHash64(visitor_id))",
        );
        l.clause("SAMPLE BY", "cityHash64(visitor_id)");
        l.clause("TTL", "event_time + INTERVAL 13 MONTH DELETE");
        l.list_clause("SETTINGS", &["ttl_only_drop_parts = 1"]);
    });
}

#[test]
fn query_clauses_of_clickhouse() {
    golden(proposal::QUERY_CLAUSES, |l| {
        l.select(&["site", "tag", "count() AS views"]);
        l.clause("FROM", "raw.articles");
        l.clause("ARRAY JOIN", "tags AS tag");
        l.clause("PREWHERE", "event_date = today()");
        l.clause("WHERE", "event_type = 'page_view'");
        let keys = l.list_clause("GROUP BY", &["site", "tag"]);
        l.b.if_break(keys, chairlift::doc::Gap::Line, chairlift::doc::Gap::Space);
        l.kw("WITH TOTALS");
        l.list_clause("ORDER BY", &["views DESC"]);
        l.clause("LIMIT", "3 BY site");
        l.clause("LIMIT", "100");
        l.list_clause("SETTINGS", &["max_threads = 8"]);
        l.clause("FORMAT", "JSONEachRow");
    });
}

#[test]
fn casts_parameters_and_except_are_atoms() {
    golden(proposal::ATOMS, |l| {
        l.select(&[
            "* EXCEPT (raw_payload)",
            "status::UInt16 AS status_code",
            "{limit:UInt32} AS row_limit",
        ]);
        l.clause("FROM", "raw.requests");
        l.clause("WHERE", "event_time >= {since:DateTime} - INTERVAL 1 DAY");
    });
}

// Verbatim text, which the proposal has no example of.

/// A window specification the parser keeps as written, re-indented as a
/// block so that its second line stays under its first.
#[test]
fn verbatim_text_is_reindented_as_a_block() {
    let source =
        "select\n  a,\n  sum(v) over (partition by k\n               order by ts) as s\nfrom t";
    let out = format_with(source, Mode::Sql, 100, &|l| {
        l.forced(|l| {
            l.t("SELECT");
            l.indent(|l| {
                l.line();
                l.t("a");
                l.t(",");
                l.line();
                l.inline("sum(v) OVER");
                l.sp();
                let first = l.take("(");
                for word in ["partition", "by", "k", "order", "by", "ts"] {
                    l.take(word);
                }
                let last = l.take(")");
                l.b.verbatim(first, last);
                l.sp();
                l.inline("AS s");
            });
        });
        l.clause("FROM", "t");
    });
    assert_eq!(
        out,
        "SELECT\n    a,\n    sum(v) OVER (partition by k\n                 order by ts) AS s\nFROM t\n"
    );
}

// Section 4: the DESIGN.md example.

fn design(l: &mut L<'_>) {
    l.comments();
    l.hard();
    l.blank();
    l.statements(3, |l, k| match k {
        0 => {
            create_table(
                l,
                "analytics.page_views_1h",
                &[
                    "hour DateTime('UTC') CODEC(Delta, ZSTD(1))",
                    "site LowCardinality(String)",
                    "views UInt64",
                    "visitors AggregateFunction(uniq, String)",
                ],
            );
            l.clause("ENGINE", "= AggregatingMergeTree");
            l.clause("PARTITION BY", "toStartOfDay(hour)");
            l.clause("ORDER BY", "(site, hour)");
            l.clause("TTL", "hour + INTERVAL 90 DAY");
            l.list_clause("SETTINGS", &["ttl_only_drop_parts = 1"]);
            l.t(";");
        }
        1 => {
            l.group(|l| {
                l.kw("CREATE MATERIALIZED VIEW");
                l.sp();
                l.inline("analytics.page_views_1h_mv");
                l.indent(|l| {
                    l.line();
                    l.t("TO");
                    l.sp();
                    l.inline("analytics.page_views_1h");
                });
                l.sp();
                l.t("AS");
            });
            l.hard();
            l.select(&[
                "toStartOfHour(event_time) AS hour",
                "site",
                "count() AS views",
                "uniqState(visitor_id) AS visitors",
            ]);
            l.clause("FROM", "raw.events");
            l.hard();
            l.t("WHERE");
            l.sp();
            let mut opener = None;
            l.group(|l| {
                l.t("true");
                l.indent(|l| {
                    l.line();
                    l.inline("AND event_type = 'page_view'");
                    opener = Some(l.tag());
                    l.inline("AND event_time >= {{ this.cutoff | sql_datetime }}");
                });
            });
            l.closer(opener.expect("an opener"));
            l.list_clause("GROUP BY", &["hour", "site"]);
            l.t(";");
        }
        _ => {
            l.group(|l| {
                l.kw("GRANT SELECT");
                l.indent(|l| {
                    l.line();
                    l.inline("ON analytics.page_views_1h");
                    l.line();
                    l.inline("TO {{ this.reader_role | sql_ident }}");
                });
            });
            l.t(";");
        }
    });
}

#[test]
fn the_design_example_prints_as_the_proposal_says() {
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/design/");
    let input = std::fs::read_to_string(format!("{fixtures}page_views_1h.sql.j2")).expect("input");
    let expected = std::fs::read_to_string(format!("{fixtures}page_views_1h.expected.sql.j2"))
        .expect("expected output");
    assert_eq!(format_with(&input, Mode::Jinja, 100, &design), expected);
    golden_at(&expected, Mode::Jinja, 100, design);
}
