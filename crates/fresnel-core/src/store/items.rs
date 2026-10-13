//! The objects of one store, with the identity of an object in its cluster.
//!
//! This module wraps the map type of `imbl`. A copy of the map shares each part that did
//! not change, so a snapshot costs no copy of the objects, and one event changes few nodes.
//! For the same reason, the differences between two snapshots cost a time that follows the
//! count of the changes and not the count of the objects.

use std::sync::Arc;

use imbl::OrdMap;
use imbl::ordmap::DiffItem;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

/// The identity of one object in the store of its kind: namespace and name. The sequence
/// is the sequence of a list by namespace and name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectKey {
    /// The namespace. Empty for an object of a kind with no namespace.
    pub namespace: Arc<str>,
    /// The name.
    pub name: Arc<str>,
}

impl ObjectKey {
    /// The identity of the object with this namespace and name.
    pub fn new(namespace: &str, name: &str) -> Self {
        Self { namespace: namespace.into(), name: name.into() }
    }

    /// The identity of an object with this metadata. `None` for an object with no name:
    /// two such objects would be one entry of a store.
    pub(crate) fn of(meta: &ObjectMeta) -> Option<Self> {
        let name = meta.name.as_deref().filter(|name| !name.is_empty())?;
        Some(Self::new(meta.namespace.as_deref().unwrap_or_default(), name))
    }
}

/// One object of the map. Two entries are equal when they are one allocation. The writer of
/// a store keeps the allocation of an object that has no new version, so the comparison of
/// two maps reads no object.
#[derive(Debug)]
struct Entry<T>(Arc<T>);

impl<T> Clone for Entry<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> PartialEq for Entry<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// One difference between the objects of a store at two moments.
#[derive(Debug)]
pub(crate) enum Change<'a, T> {
    /// The later moment has one more object.
    Added(&'a ObjectKey, &'a Arc<T>),
    /// The object is gone at the later moment.
    Removed(&'a ObjectKey, &'a Arc<T>),
    /// The object has a new version, or a new object has its name.
    Updated { key: &'a ObjectKey, old: &'a Arc<T>, new: &'a Arc<T> },
}

/// The objects of one store at one moment, in the sequence of their identities.
#[derive(Debug)]
pub struct Items<T> {
    map: OrdMap<ObjectKey, Entry<T>>,
}

impl<T> Clone for Items<T> {
    /// The copy shares each object and each node of the map.
    fn clone(&self) -> Self {
        Self { map: self.map.clone() }
    }
}

impl<T> Default for Items<T> {
    fn default() -> Self {
        Self { map: OrdMap::new() }
    }
}

impl<T> Items<T> {
    /// The count of objects.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True with no object.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The object with this identity.
    pub fn get(&self, key: &ObjectKey) -> Option<&Arc<T>> {
        self.map.get(key).map(|entry| &entry.0)
    }

    /// Each object with its identity, in the sequence of the identities.
    pub fn iter(&self) -> impl Iterator<Item = (&ObjectKey, &Arc<T>)> {
        self.map.iter().map(|(key, entry)| (key, &entry.0))
    }

    /// Each difference from these objects to the objects of a later moment, in the sequence
    /// of the identities. An index follows a store with it: a difference cannot lose a
    /// change, as a missed event can. The walk does not enter a part that the two maps share.
    pub(crate) fn changes_to<'a>(&'a self, later: &'a Self) -> impl Iterator<Item = Change<'a, T>> {
        self.map.diff(&later.map).map(|difference| match difference {
            DiffItem::Add(key, new) => Change::Added(key, &new.0),
            DiffItem::Remove(key, old) => Change::Removed(key, &old.0),
            DiffItem::Update { old: (key, old), new: (_, new) } => Change::Updated { key, old: &old.0, new: &new.0 },
        })
    }

    pub(super) fn insert(&mut self, key: ObjectKey, object: Arc<T>) {
        self.map.insert(key, Entry(object));
    }

    /// True when the map had the object.
    pub(super) fn remove(&mut self, key: &ObjectKey) -> bool {
        self.map.remove(key).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> ObjectKey {
        ObjectKey::new("shop", name)
    }

    fn words<'a>(changes: impl Iterator<Item = Change<'a, &'static str>>) -> Vec<String> {
        let word = |change| match change {
            Change::Added(key, new) => format!("+{} {new}", key.name),
            Change::Removed(key, old) => format!("-{} {old}", key.name),
            Change::Updated { key, old, new } => format!("{} {old}>{new}", key.name),
        };
        changes.map(word).collect()
    }

    #[test]
    fn the_differences_of_two_moments_are_the_objects_with_a_new_allocation() {
        let mut before = Items::default();
        let same = Arc::new("v1");
        before.insert(key("a"), Arc::clone(&same));
        before.insert(key("b"), Arc::new("v1"));
        before.insert(key("c"), Arc::new("v1"));
        assert_eq!(words(before.changes_to(&before)), Vec::<String>::new());

        let mut after = before.clone();
        // The same allocation again is no change. An equal value in a new one is a change:
        // the store gives a new allocation only to a new version.
        after.insert(key("a"), same);
        after.insert(key("b"), Arc::new("v1"));
        after.remove(&key("c"));
        after.insert(key("d"), Arc::new("v1"));
        assert_eq!(words(before.changes_to(&after)), ["b v1>v1", "-c v1", "+d v1"]);
        assert_eq!(words(after.changes_to(&before)), ["b v1>v1", "+c v1", "-d v1"]);
        let empty = Items::default();
        assert_eq!(words(empty.changes_to(&before)), ["+a v1", "+b v1", "+c v1"], "the first read of a store");
    }
}
