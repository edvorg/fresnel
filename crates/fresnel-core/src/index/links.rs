//! One relation of an index: from a key to the set of what belongs to it.
//!
//! This module wraps the map types of `imbl` for the indexes. A copy shares each part that
//! did not change. Thus a new value of an index costs no copy of its relations, and a
//! reader can hold an old value with little memory.

use std::borrow::Borrow;

use imbl::{OrdMap, OrdSet};

pub(super) struct Links<K, V> {
    map: OrdMap<K, OrdSet<V>>,
}

impl<K, V> Clone for Links<K, V> {
    /// The copy shares each node of the map.
    fn clone(&self) -> Self {
        Self { map: self.map.clone() }
    }
}

impl<K, V> Default for Links<K, V> {
    fn default() -> Self {
        Self { map: OrdMap::new() }
    }
}

impl<K: Ord + Clone, V: Ord + Clone> Links<K, V> {
    /// Adds one link. True when the relation did not have it.
    pub(super) fn add(&mut self, from: K, to: V) -> bool {
        self.map.entry(from).or_default().insert(to).is_none()
    }

    /// Removes one link. True when the relation had it. A key with no link leaves the map.
    pub(super) fn remove<Q>(&mut self, from: &Q, to: &V) -> bool
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let Some(links) = self.map.get_mut(from) else {
            return false;
        };
        let removed = links.remove(to).is_some();
        if links.is_empty() {
            self.map.remove(from);
        }
        removed
    }

    /// What belongs to this key, in its sequence. The result borrows the relation only,
    /// so a caller can give a key that it made for the call.
    pub(super) fn of<'a, Q>(&'a self, from: &Q) -> impl Iterator<Item = &'a V> + use<'a, K, V, Q>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.map.get(from).into_iter().flatten()
    }

    /// The count of links and their bytes: each key and each value in the nodes of the map,
    /// with no text, which the keys share with the stores (the memory account).
    pub(super) fn size(&self) -> (usize, usize) {
        let links: usize = self.map.values().map(OrdSet::len).sum();
        (links, self.map.len() * size_of::<(K, OrdSet<V>)>() + links * size_of::<V>())
    }

    /// The count of keys with a link.
    #[cfg(test)]
    pub(super) fn keys(&self) -> usize {
        self.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_in_the_relation_one_time_and_an_empty_key_leaves_it() {
        let mut links = Links::<String, u32>::default();
        assert!(links.add("a".to_owned(), 2) && links.add("a".to_owned(), 1) && links.add("b".to_owned(), 7));
        assert!(!links.add("a".to_owned(), 2), "the relation has this link");
        let before = links.clone();
        assert_eq!(links.of("a").copied().collect::<Vec<_>>(), [1, 2], "in the sequence of the values");

        assert!(links.remove("a", &1) && !links.remove("a", &1) && !links.remove("c", &1));
        assert!(links.remove("a", &2));
        assert_eq!((links.of("a").count(), links.keys()), (0, 1));
        assert_eq!((before.of("a").count(), before.keys()), (2, 2), "a copy does not change");
    }
}
