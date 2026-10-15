//! A text to find in the lines of a log view: the search and the filter.
//!
//! A text with no capital letter ignores the case of ASCII letters, and a text with one
//! keeps it ("smart case" of `less` and ripgrep): one field and no switch. A letter that is
//! not ASCII matches as it is. The search reads the text of a line with no escape sequence,
//! so a color code inside a word does not hide the word.

use std::ops::Range;
use std::sync::Arc;

/// A text to find.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Needle {
    /// In small letters when the case does not matter.
    text: Arc<str>,
    exact: bool,
}

impl Needle {
    /// The needle of the text of a field. `None` for an empty text: it finds nothing.
    pub(crate) fn new(text: &str) -> Option<Self> {
        if text.is_empty() {
            return None;
        }
        let exact = text.bytes().any(|byte| byte.is_ascii_uppercase());
        let text = if exact { text.into() } else { text.to_ascii_lowercase().into() };
        Some(Self { text, exact })
    }

    /// True when the text has the needle.
    pub(crate) fn is_in(&self, text: &str) -> bool {
        self.find(text.as_bytes(), 0).is_some()
    }

    /// The bytes of each place of the needle in the first `end` bytes of a text. The places
    /// do not overlap, and each one starts and ends at the start of a character: a needle
    /// is a whole text, and UTF-8 never has an ASCII byte inside a character.
    pub(crate) fn places(&self, text: &str, end: usize) -> Vec<Range<usize>> {
        let hay = text.as_bytes().get(..end.min(text.len())).unwrap_or_default();
        let mut places = Vec::new();
        let mut from = 0;
        while let Some(at) = self.find(hay, from) {
            let to = at + self.text.len();
            places.push(at..to);
            from = to;
        }
        places
    }

    /// The first place of the needle at or after `from`.
    fn find(&self, hay: &[u8], from: usize) -> Option<usize> {
        let needle = self.text.as_bytes();
        let first = *needle.first()?;
        let last = hay.len().checked_sub(needle.len())?;
        let mut at = from;
        while at <= last {
            let rest = hay.get(at..=last)?;
            let skip = if self.exact {
                rest.iter().position(|byte| *byte == first)?
            } else {
                rest.iter().position(|byte| byte.to_ascii_lowercase() == first)?
            };
            at += skip;
            let window = hay.get(at..at + needle.len())?;
            if if self.exact { window == needle } else { window.eq_ignore_ascii_case(needle) } {
                return Some(at);
            }
            at += 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn needle(text: &str) -> Needle {
        Needle::new(text).unwrap_or_else(|| panic!("a needle for {text:?}"))
    }

    #[test]
    fn a_text_in_small_letters_ignores_the_case_and_a_capital_letter_keeps_it() {
        assert!(needle("error").is_in("an ERROR here") && needle("error").is_in("Error"));
        assert!(!needle("Error").is_in("an ERROR here") && needle("Error").is_in("an Error"));
        assert!(needle("größe").is_in("die größe") && !needle("größe").is_in("die GRÖSSE"));
        assert_eq!(Needle::new(""), None);
        assert!(!needle("long needle").is_in("short"));
    }

    #[test]
    fn the_places_of_a_needle_do_not_overlap_and_stay_in_their_part() {
        assert_eq!(needle("aa").places("aaaaa", 5), [0..2, 2..4]);
        assert_eq!(needle("ab").places("xabyab", 5), vec![1..3], "the second place ends after the part");
        assert_eq!(needle("ß").places("ßaß", 10), [0..2, 3..5]);
        assert_eq!(needle("x").places("", 3), Vec::<Range<usize>>::new());
    }
}
