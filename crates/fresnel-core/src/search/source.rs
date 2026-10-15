//! The objects of one store as the quick search reads them, with no type of their objects.

use std::any::Any;
use std::sync::Arc;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::score::Matcher;
use crate::cluster::ClusterId;
use crate::kinds::{Describe, Kind, Meta, Object};
use crate::published::Held;
use crate::store::{ObjectKey, Snapshot, StoreStatus};

/// The count of objects between two reads of the cancel mark.
const CANCEL_STEP: usize = 1024;

/// An object type of a store, with the texts that the search matches besides the metadata:
/// the images, the nodes and the addresses of a full object, and nothing of the metadata
/// of a tier B store.
pub(crate) trait Searched: Object {
    fn texts<'a>(&'a self, texts: &mut Vec<&'a str>);
}

impl<K: Describe> Searched for K {
    fn texts<'a>(&'a self, texts: &mut Vec<&'a str>) {
        self.search(texts);
    }
}

impl Searched for Meta {
    fn texts<'a>(&'a self, _texts: &mut Vec<&'a str>) {}
}

/// The snapshot of one store with no type of its objects.
trait Searchable: Send + Sync {
    fn as_any(&self) -> &dyn Any;

    /// The count of objects in one namespace.
    fn count_in(&self, namespace: &str) -> usize;

    /// Gives the metadata of each object.
    fn each_meta(&self, visit: &mut dyn FnMut(&ObjectKey, &ObjectMeta));

    /// Gives each object that matches, with its score. False when `cancel` stopped the walk.
    fn find<'s>(
        &'s self,
        matcher: &Matcher,
        cancel: &dyn Fn() -> bool,
        hit: &mut dyn FnMut(&'s ObjectKey, u32),
    ) -> bool;
}

impl<T: Searched> Searchable for Held<Snapshot<T>> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn count_in(&self, namespace: &str) -> usize {
        self.items.count_in(namespace)
    }

    fn each_meta(&self, visit: &mut dyn FnMut(&ObjectKey, &ObjectMeta)) {
        self.items.iter().for_each(|(key, object)| visit(key, object.meta()));
    }

    fn find<'s>(
        &'s self,
        matcher: &Matcher,
        cancel: &dyn Fn() -> bool,
        hit: &mut dyn FnMut(&'s ObjectKey, u32),
    ) -> bool {
        let mut texts = Vec::new();
        for (count, (key, object)) in self.items.iter().enumerate() {
            if count.is_multiple_of(CANCEL_STEP) && cancel() {
                return false;
            }
            texts.clear();
            if let Some(score) = matcher.score(key, object.meta(), |texts| object.texts(texts), &mut texts) {
                hit(key, score);
            }
        }
        true
    }
}

/// One store of one cluster at one moment: its facts and its objects. A copy shares the
/// snapshot.
#[derive(Clone)]
pub struct Source {
    pub status: StoreStatus,
    objects: Arc<dyn Searchable>,
}

impl Source {
    /// The full objects of a kind.
    pub fn full<K: Describe>(snapshot: Held<Snapshot<K>>) -> Self {
        Self::of(K::ID, snapshot)
    }

    /// The metadata of the objects of a kind of tier B.
    pub fn metadata(kind: Kind, snapshot: Held<Snapshot<Meta>>) -> Self {
        Self::of(kind, snapshot)
    }

    /// The snapshot of the store of a kind, with the objects of its type.
    pub(crate) fn of<T: Searched>(kind: Kind, snapshot: Held<Snapshot<T>>) -> Self {
        Self { status: snapshot.status(kind), objects: Arc::new(snapshot) }
    }

    /// The full objects of a kind, with their type. `None` for a store of the metadata.
    pub fn snapshot<K: Describe>(&self) -> Option<&Held<Snapshot<K>>> {
        self.objects.as_any().downcast_ref()
    }

    /// The count of objects of the store in one namespace: a view of a namespace counts
    /// its objects of each kind, with no type of their objects.
    pub fn count_in(&self, namespace: &str) -> usize {
        self.objects.count_in(namespace)
    }

    /// Gives the metadata of each object, in the sequence of the store, with no type of the
    /// objects: the soak of `fresnel-cli` compares the UID and the version of each one with
    /// a list of the API.
    pub fn each_meta(&self, visit: &mut dyn FnMut(&ObjectKey, &ObjectMeta)) {
        self.objects.each_meta(visit);
    }

    pub(super) fn find<'s>(
        &'s self,
        matcher: &Matcher,
        cancel: &dyn Fn() -> bool,
        hit: &mut dyn FnMut(&'s ObjectKey, u32),
    ) -> bool {
        self.objects.find(matcher, cancel, hit)
    }
}

impl std::fmt::Debug for Source {
    /// The facts, with no object.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Source").field("status", &self.status).finish_non_exhaustive()
    }
}

/// What the search reads of one selected cluster: the store of each kind that it has, or
/// why it has no stores.
#[derive(Clone, Debug)]
pub struct ClusterSources {
    pub cluster: ClusterId,
    /// The stores, one for each kind at most. The reason when the cluster has none: its
    /// connect did not start, or it starts.
    pub sources: Result<Vec<Source>, String>,
}
