//! The objects of one store, with the identity of an object in its cluster.
//!
//! This module wraps the map type of `imbl`. A copy of the map shares each part that did
//! not change, so a snapshot costs no copy of the objects, and one event changes few nodes.
//! For the same reason, the differences between two snapshots cost a time that follows the
//! count of the changes and not the count of the objects.

use std::hint::black_box;
use std::sync::Arc;

use imbl::OrdMap;
use imbl::ordmap::DiffItem;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use crate::kinds::Object;
use crate::memory::Account;

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
pub enum Change<'a, T> {
    /// The later moment has one more object.
    Added(&'a ObjectKey, &'a Arc<T>),
    /// The object is gone at the later moment.
    Removed(&'a ObjectKey, &'a Arc<T>),
    /// The object has a new version, or a new object has its name.
    Updated {
        /// The identity of the object in its store.
        key: &'a ObjectKey,
        /// The object at the earlier moment.
        old: &'a Arc<T>,
        /// The object at the later moment.
        new: &'a Arc<T>,
    },
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

    /// The count of objects in one namespace. The map holds the objects of a namespace in
    /// one run, so the count reads no object of an other namespace.
    pub fn count_in(&self, namespace: &str) -> usize {
        let first = ObjectKey::new(namespace, "");
        self.map.range(first..).take_while(|(key, _)| &*key.namespace == namespace).count()
    }

    /// Each object with its identity, in the sequence of the identities.
    pub fn iter(&self) -> impl Iterator<Item = (&ObjectKey, &Arc<T>)> {
        self.map.iter().map(|(key, entry)| (key, &entry.0))
    }

    /// Each difference from these objects to the objects of a later moment, in the sequence
    /// of the identities. An index and a view follow a store with it: a difference cannot
    /// lose a change, as a missed event can. The walk does not enter a part that the two
    /// maps share, so two moments with the same objects cost nothing.
    pub fn changes_to<'a>(&'a self, later: &'a Self) -> impl Iterator<Item = Change<'a, T>> {
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

impl<T: Clone> Items<T> {
    /// The bytes of the objects (the memory account): a copy of each object and of its key
    /// by the scale of the account, and for each one its shared pointer and its entry in
    /// the map. A copy that a reader holds shares these bytes.
    pub(crate) fn bytes(&self, account: &Account) -> usize {
        let copies = account.measure(&mut || {
            for (key, object) in self.iter() {
                black_box((ObjectKey::new(&key.namespace, &key.name), T::clone(object)));
            }
        });
        let each = size_of::<(ObjectKey, Entry<T>)>() + 2 * size_of::<usize>() + size_of::<T>();
        copies + self.len() * each
    }
}

impl<T: Object> FromIterator<Arc<T>> for Items<T> {
    /// The objects of a list as a store holds them. A test of a reader builds the snapshot
    /// of each state with it. An object with no name is not an entry, as in a store.
    fn from_iter<I: IntoIterator<Item = Arc<T>>>(objects: I) -> Self {
        let mut items = Self::default();
        for object in objects {
            if let Some(key) = ObjectKey::of(object.meta()) {
                items.insert(key, object);
            }
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::Meta;

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

    #[test]
    fn a_list_of_objects_gives_the_entries_of_a_store_and_an_object_with_no_name_is_none_and_each_namespace_counts() {
        let named = |namespace: &str, name: &str| {
            let mut meta = Meta::default();
            meta.metadata.namespace = Some(namespace.to_owned());
            meta.metadata.name = Some(name.to_owned());
            Arc::new(meta)
        };
        let items: Items<Meta> =
            [named("shop", "web"), named("lab", "web"), Arc::new(Meta::default()), named("shop", "api")]
                .into_iter()
                .collect();
        let keys: Vec<String> = items.iter().map(|(key, _)| format!("{}/{}", key.namespace, key.name)).collect();
        assert_eq!(keys, ["lab/web", "shop/api", "shop/web"]);
        let counts = ["", "lab", "sho", "shop", "shop2"].map(|namespace| items.count_in(namespace));
        assert_eq!(counts, [0, 1, 0, 2, 0], "a namespace counts its own objects only");
    }

    /// An object that counts its copies.
    struct Counted(Arc<std::sync::atomic::AtomicUsize>);

    impl Clone for Counted {
        fn clone(&self) -> Self {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Self(Arc::clone(&self.0))
        }
    }

    /// The scale of the test: 100 bytes for each copy of the walk.
    struct Hundred;

    impl crate::memory::Scale for Hundred {
        fn bytes(&self, copy: &mut dyn FnMut()) -> usize {
            copy();
            100
        }
    }

    #[test]
    fn the_bytes_of_the_objects_are_the_measure_of_a_copy_of_each_one_and_their_entries() {
        let copies = Arc::default();
        let mut items = Items::default();
        for name in ["a", "b", "c"] {
            items.insert(key(name), Arc::new(Counted(Arc::clone(&copies))));
        }
        let each = size_of::<(ObjectKey, Entry<Counted>)>() + 2 * size_of::<usize>() + size_of::<Counted>();
        assert_eq!(items.bytes(&Account::new(&Hundred)), 100 + 3 * each);
        assert_eq!(copies.load(std::sync::atomic::Ordering::Relaxed), 3, "a copy of each object");
    }
}
