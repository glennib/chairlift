//! ClickHouse's parser keywords.
//!
//! The list is generated from `APPLY_FOR_PARSER_KEYWORDS` and
//! `APPLY_FOR_PARSER_KEYWORDS_WITH_UNDERSCORES` in ClickHouse's
//! `src/Parsers/CommonParsers.h` by `scripts/generate-keywords.sh`, for the
//! release in [`CLICKHOUSE_VERSION`](crate::CLICKHOUSE_VERSION).
//! ClickHouse's parser matches keywords as sequences of bare words compared
//! case-insensitively; a [`Keyword`] is one such word, and [`Keyword::phrases`]
//! lists the sequences.
//!
//! A word being a keyword is necessary but not sufficient for re-casing it:
//! ClickHouse has no reserved identifiers, so `key`, `date`, `order` or `from`
//! are valid column names, and only the parser knows whether a word stands in
//! a keyword slot.
//!
//! ```
//! use chairlift::keyword::Keyword;
//!
//! assert_eq!(Keyword::from_word("select"), Some(Keyword::Select));
//! assert_eq!(Keyword::from_word("Select").map(Keyword::as_str), Some("SELECT"));
//! assert_eq!(Keyword::from_word("toStartOfHour"), None);
//! ```

mod generated;

pub use generated::Keyword;

impl Keyword {
    /// The keyword's text in upper case, as ClickHouse's parser matches it
    /// (case-insensitively).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        generated::TEXT[self as usize]
    }

    /// The keyword `word` spells, compared case-insensitively, or `None` if it is
    /// not one.
    ///
    /// `word` is the text of a single bare word; anything containing characters
    /// other than ASCII letters, digits and `_` is never a keyword.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Keyword> {
        let bytes = word.as_bytes();
        if bytes.is_empty() || bytes.len() > generated::MAX_LEN {
            return None;
        }
        let mut upper = [0u8; generated::MAX_LEN];
        for (dst, &b) in upper.iter_mut().zip(bytes) {
            if !(b.is_ascii_alphanumeric() || b == b'_') {
                return None;
            }
            *dst = b.to_ascii_uppercase();
        }
        let key = &upper[..bytes.len()];
        generated::BY_LENGTH
            .binary_search_by(|k| {
                let text = k.as_str().as_bytes();
                text.len().cmp(&key.len()).then_with(|| text.cmp(key))
            })
            .ok()
            .map(|i| generated::BY_LENGTH[i])
    }

    /// ClickHouse's keyword phrases, single words and sequences such as
    /// `ORDER BY` or `ADD COLUMN`, in the order of ClickHouse's `Keyword` enum.
    ///
    /// ```
    /// use chairlift::keyword::Keyword;
    ///
    /// assert!(Keyword::phrases().contains(&&[Keyword::Order, Keyword::By][..]));
    /// ```
    #[must_use]
    pub fn phrases() -> &'static [&'static [Keyword]] {
        &generated::PHRASES
    }

    /// The ClickHouse release the keyword list was generated from.
    pub const SOURCE_VERSION: &'static str = generated::SOURCE_VERSION;
}

impl std::fmt::Display for Keyword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_from_the_targeted_release() {
        assert_eq!(Keyword::SOURCE_VERSION, crate::CLICKHOUSE_VERSION);
    }

    #[test]
    fn every_keyword_round_trips_through_its_text() {
        for keyword in Keyword::ALL {
            assert_eq!(Keyword::from_word(keyword.as_str()), Some(keyword));
            assert_eq!(
                Keyword::from_word(&keyword.as_str().to_ascii_lowercase()),
                Some(keyword)
            );
        }
    }

    #[test]
    fn discriminants_index_the_text_table() {
        for (i, keyword) in Keyword::ALL.into_iter().enumerate() {
            assert_eq!(keyword as usize, i);
        }
        assert!(
            Keyword::ALL
                .windows(2)
                .all(|w| w[0].as_str() < w[1].as_str())
        );
    }

    #[test]
    fn non_keywords() {
        for word in [
            "",
            "toStartOfHour",
            "events_{{",
            "sélect",
            "SELECTS",
            "SELEC",
            "a b",
            "ORDER BY",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            assert_eq!(Keyword::from_word(word), None, "{word:?}");
        }
    }

    #[test]
    fn mixed_case_entries_are_upper_cased() {
        assert_eq!(Keyword::from_word("protobuf"), Some(Keyword::Protobuf));
        assert_eq!(Keyword::Protobuf.as_str(), "PROTOBUF");
        assert_eq!(Keyword::BaseBackup.as_str(), "BASE_BACKUP");
    }
}
