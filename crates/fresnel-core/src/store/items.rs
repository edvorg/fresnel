//! The objects of one store, with the identity of an object in its cluster.
//!
//! This module wraps the map type of `imbl`. A copy of the map shares each part that did
//! not change, so a snapshot costs no copy of the objects, and one event changes few nodes.

use std::sync::Arc;

use imbl::OrdMap;
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

/// The objects of one store at one moment, in the sequence of their identities.
#[derive(Debug)]
pub struct Items<T> {
    map: OrdMap<ObjectKey, Arc<T>>,
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
        self.map.get(key)
    }

    /// Each object with its identity, in the sequence of the identities.
    pub fn iter(&self) -> impl Iterator<Item = (&ObjectKey, &Arc<T>)> {
        self.map.iter()
    }

    pub(super) fn insert(&mut self, key: ObjectKey, object: Arc<T>) {
        self.map.insert(key, object);
    }

    /// True when the map had the object.
    pub(super) fn remove(&mut self, key: &ObjectKey) -> bool {
        self.map.remove(key).is_some()
    }
}
