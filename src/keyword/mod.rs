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
        let mut hash = FNV_OFFSET;
        for (dst, &b) in upper.iter_mut().zip(bytes) {
            if !(b.is_ascii_alphanumeric() || b == b'_') {
                return None;
            }
            *dst = b.to_ascii_uppercase();
            hash = fnv_step(hash, *dst);
        }
        let key = &upper[..bytes.len()];
        let mut slot = slot_of(hash);
        loop {
            let index = TABLE[slot];
            if index == EMPTY {
                return None;
            }
            let keyword = Keyword::ALL[usize::from(index)];
            if keyword.as_str().as_bytes() == key {
                return Some(keyword);
            }
            slot = (slot + 1) % SLOTS;
        }
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

/// The lookup table for [`Keyword::from_word`]: an open-addressing hash table
/// over the keywords' upper-case text (FNV-1a, linear probing), built at
/// compile time. Every bare word the lexer sees is looked up, so this is on
/// the lexer's hot path.
static TABLE: [u16; SLOTS] = build_table();

/// Four times the number of keywords, rounded up to a power of two, so that
/// probe sequences stay short.
const SLOTS: usize = (generated::TEXT.len() * 4).next_power_of_two();
const EMPTY: u16 = u16::MAX;
const FNV_OFFSET: u32 = 0x811c_9dc5;

const fn fnv_step(hash: u32, byte: u8) -> u32 {
    (hash ^ byte as u32).wrapping_mul(0x0100_0193)
}

const fn slot_of(hash: u32) -> usize {
    hash as usize % SLOTS
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "there are fewer keywords than u16::MAX"
)]
const fn build_table() -> [u16; SLOTS] {
    let mut table = [EMPTY; SLOTS];
    let mut index = 0;
    while index < generated::TEXT.len() {
        let text = generated::TEXT[index].as_bytes();
        let mut hash = FNV_OFFSET;
        let mut i = 0;
        while i < text.len() {
            hash = fnv_step(hash, text[i]);
            i += 1;
        }
        let mut slot = slot_of(hash);
        while table[slot] != EMPTY {
            slot = (slot + 1) % SLOTS;
        }
        table[slot] = index as u16;
        index += 1;
    }
    table
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
