//! The match of one object (plan section 12.1): a fuzzy match on the name, and a text match
//! on the namespace, the labels, the owners and the texts of its kind (images, nodes and
//! addresses).
//!
//! The text of the user has words. Each word must match the name or one of the other
//! texts, so `shop web` finds the pod `web-1` in the namespace `shop`. A word that matches
//! the name gives more score than one that matches an other text, and a closer match of
//! the name gives more than a looser one. Kubernetes names are ASCII, so the comparison
//! ignores the case of ASCII letters and needs no copy of a text.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use crate::store::ObjectKey;

/// The score of a word that is the name.
const EXACT: u32 = 1000;
/// The score of a word at the start of the name.
const PREFIX: u32 = 800;
/// The score of a word in the name. A word at the start of a part of the name gets more.
const PART: u32 = 600;
const PART_START: u32 = 100;
/// The scores of the letters of a word that are in the name in their sequence, with gaps:
/// the lowest name match, which stays below a match of the full word.
const LETTERS: u32 = 100;
const LETTERS_MAX: u32 = 590;
/// The score of a word that matches an other text of the object.
const FIELD: u32 = 50;

/// The words of a search.
pub(crate) struct Matcher {
    words: Vec<Vec<u8>>,
}

impl Matcher {
    /// The words of a text, in small letters. A text with no word matches nothing.
    pub(crate) fn new(text: &str) -> Self {
        Self { words: text.split_whitespace().map(|word| word.to_ascii_lowercase().into_bytes()).collect() }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// The score of an object, `None` when a word matches no text of it. `extra` gives
    /// the texts of its kind into `texts`; it runs only for a word that the name lacks.
    pub(crate) fn score<'a>(
        &self,
        key: &'a ObjectKey,
        meta: &'a ObjectMeta,
        extra: impl FnOnce(&mut Vec<&'a str>),
        texts: &mut Vec<&'a str>,
    ) -> Option<u32> {
        if self.words.is_empty() {
            return None;
        }
        let mut extra = Some(extra);
        let mut total: u32 = 0;
        for word in &self.words {
            // The sum stays at the largest score: a text of millions of words is no
            // reason for a panic of the worker.
            if let Some(score) = name_score(key.name.as_bytes(), word) {
                total = total.saturating_add(score);
                continue;
            }
            if let Some(extra) = extra.take() {
                texts.push(&*key.namespace);
                texts.extend(meta.owner_references.iter().flatten().map(|owner| owner.name.as_str()));
                extra(texts);
            }
            let labels = meta.labels.iter().flatten();
            let in_label = labels.into_iter().any(|(label, value)| label_matches(label, value, word));
            if !in_label && !texts.iter().any(|text| contains(text.as_bytes(), word)) {
                return None;
            }
            total = total.saturating_add(FIELD);
        }
        Some(total)
    }
}

/// The score of a word in a name. `None` when the letters of the word are not in the
/// name in their sequence.
fn name_score(name: &[u8], word: &[u8]) -> Option<u32> {
    if name.eq_ignore_ascii_case(word) {
        return Some(EXACT);
    }
    if name.get(..word.len()).is_some_and(|start| start.eq_ignore_ascii_case(word)) {
        return Some(PREFIX);
    }
    if let Some(at) = find(name, word) {
        let part_start = at.checked_sub(1).and_then(|before| name.get(before)).is_some_and(|byte| is_break(*byte));
        return Some(PART + if part_start { PART_START } else { 0 });
    }
    letters(name, word)
}

/// The letters of the word in the name, in their sequence, from the left. A letter at the
/// start of a part of the name and a letter right after the one before score more.
fn letters(name: &[u8], word: &[u8]) -> Option<u32> {
    let (mut score, mut from, mut last) = (LETTERS, 0, None);
    for wanted in word {
        let rest = name.get(from..)?;
        let at = from + rest.iter().position(|byte| byte.eq_ignore_ascii_case(wanted))?;
        score += 10;
        if at == 0 || at.checked_sub(1).and_then(|before| name.get(before)).is_some_and(|byte| is_break(*byte)) {
            score += 15;
        }
        if last.is_some_and(|last: usize| last + 1 == at) {
            score += 10;
        }
        (last, from) = (Some(at), at + 1);
    }
    Some(score.min(LETTERS_MAX))
}

/// A sign between two parts of a name.
fn is_break(byte: u8) -> bool {
    matches!(byte, b'-' | b'.' | b'_' | b'/' | b':')
}

/// A label matches a word in the label or in its value. A word `key=value` matches a
/// label whose key has the part before `=` and whose value has the part after it.
fn label_matches(label: &str, value: &str, word: &[u8]) -> bool {
    match word.iter().position(|byte| *byte == b'=') {
        Some(at) => {
            let (key_part, value_part) = (word.get(..at).unwrap_or_default(), word.get(at + 1..).unwrap_or_default());
            contains(label.as_bytes(), key_part) && contains(value.as_bytes(), value_part)
        }
        None => contains(label.as_bytes(), word) || contains(value.as_bytes(), word),
    }
}

/// True when the text has the word, with no regard to the case of ASCII letters.
fn contains(text: &[u8], word: &[u8]) -> bool {
    word.is_empty() || find(text, word).is_some()
}

fn find(text: &[u8], word: &[u8]) -> Option<usize> {
    text.windows(word.len().max(1)).position(|window| window.eq_ignore_ascii_case(word))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;

    use super::*;

    fn score(text: &str, name: &str, meta: &ObjectMeta, extra: &[&str]) -> Option<u32> {
        let key = ObjectKey::new(meta.namespace.as_deref().unwrap_or_default(), name);
        let mut texts = Vec::new();
        Matcher::new(text).score(&key, meta, |texts| texts.extend(extra.iter().copied()), &mut texts)
    }

    fn meta() -> ObjectMeta {
        ObjectMeta {
            namespace: Some("shop".to_owned()),
            labels: Some(BTreeMap::from([("app".to_owned(), "web".to_owned())])),
            owner_references: Some(vec![OwnerReference { name: "web-5d8f".to_owned(), ..OwnerReference::default() }]),
            ..ObjectMeta::default()
        }
    }

    #[test]
    fn a_closer_match_of_the_name_scores_more() {
        let meta = ObjectMeta::default();
        let of = |text, name| score(text, name, &meta, &[]);
        let ranks = [
            of("web", "web"),
            of("web", "web-1"),
            of("web", "shop-web-1"),
            of("web", "theweb"),
            of("wb1", "web-1"),
            of("wb1", "wxxxbxxx1"),
        ];
        let values: Vec<u32> = ranks.iter().map(|rank| rank.expect("a match")).collect();
        assert!(values.is_sorted_by(|a, b| a > b), "each one scores less than the one before: {values:?}");
        assert_eq!(of("WEB", "Web-1"), of("web", "web-1"), "the case of a letter does not count");
        assert_eq!(of("bew", "web-1"), None, "the letters in their sequence only");
        assert_eq!(of("", "web"), None, "no word matches nothing");
    }

    #[test]
    fn each_word_must_match_the_name_or_an_other_text() {
        let meta = meta();
        assert!(score("shop web", "web-1", &meta, &[]).is_some(), "the namespace and the name");
        assert!(score("app=web", "db-1", &meta, &[]).is_some(), "a label as key=value");
        assert!(score("app=db", "db-1", &meta, &[]).is_none(), "the value of the label differs");
        assert!(score("5d8f", "web-1", &meta, &[]).is_some(), "the name of an owner");
        assert!(score("nginx:1.27", "web-1", &meta, &["docker.io/nginx:1.27", "10.42.0.7"]).is_some(), "an image");
        assert!(score("10.42.0", "web-1", &meta, &["docker.io/nginx:1.27", "10.42.0.7"]).is_some(), "an address");
        assert_eq!(score("shop nothing", "web-1", &meta, &[]), None, "a word that matches no text");
        let name = score("web", "web-1", &meta, &[]).expect("a match");
        let field = score("shop", "web-1", &meta, &[]).expect("a match");
        assert!(name > field, "a match of the name scores more than a match of an other text");
    }

    /// The texts of the kind cost a walk through the object, so they come only for a word
    /// that the name lacks.
    #[test]
    fn the_texts_of_the_kind_come_only_for_a_word_that_the_name_lacks() {
        let (meta, key) = (meta(), ObjectKey::new("shop", "web-1"));
        let mut texts = Vec::new();
        let asked = std::cell::Cell::new(false);
        let mark = |_: &mut Vec<&str>| asked.set(true);
        assert!(Matcher::new("web").score(&key, &meta, mark, &mut texts).is_some());
        assert!(!asked.get());
        assert!(Matcher::new("web shop").score(&key, &meta, mark, &mut texts).is_some());
        assert!(asked.get());
    }
}
