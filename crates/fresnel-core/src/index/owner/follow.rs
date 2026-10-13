//! The write side of the owner index: it applies the changes of its four stores.

use std::sync::Arc;

use k8s_openapi::api::apps::v1::ReplicaSet;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::discovery::v1::EndpointSlice;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::{ObjectRef, Owners};
use crate::index::links::Links;
use crate::index::source::Tracked;
use crate::index::task::{Indexer, Read};
use crate::kinds::{Kind, Object};
use crate::published::Reader;
use crate::store::{Change, ObjectKey, Snapshot};

/// The label of an EndpointSlice with the name of its Service.
const SERVICE_NAME: &str = "kubernetes.io/service-name";

pub(in crate::index) struct OwnerIndexer {
    relations: Owners,
    replica_sets: Tracked<ReplicaSet>,
    jobs: Tracked<Job>,
    pods: Tracked<Pod>,
    slices: Tracked<EndpointSlice>,
}

impl OwnerIndexer {
    pub(in crate::index) fn new(
        replica_sets: Reader<Snapshot<ReplicaSet>>,
        jobs: Reader<Snapshot<Job>>,
        pods: Reader<Snapshot<Pod>>,
        slices: Reader<Snapshot<EndpointSlice>>,
    ) -> Self {
        Self {
            relations: Owners::default(),
            replica_sets: Tracked::new(Kind::ReplicaSet, replica_sets),
            jobs: Tracked::new(Kind::Job, jobs),
            pods: Tracked::new(Kind::Pod, pods),
            slices: Tracked::new(Kind::EndpointSlice, slices),
        }
    }
}

impl Indexer for OwnerIndexer {
    type Relations = Owners;

    fn read(&mut self) -> Read {
        let Owners { children, slices, endpoints } = &mut self.relations;
        let mut changed = false;
        let sources = vec![
            self.replica_sets.read(|change| changed |= follow_owned(children, Kind::ReplicaSet, change)),
            self.jobs.read(|change| changed |= follow_owned(children, Kind::Job, change)),
            self.pods.read(|change| changed |= follow_owned(children, Kind::Pod, change)),
            self.slices.read(|change| changed |= follow_slice(slices, change)),
        ];
        *endpoints = self.slices.seen().clone();
        Read { changed, sources }
    }

    fn relations(&self) -> Owners {
        self.relations.clone()
    }

    async fn changed(&mut self) {
        tokio::select! {
            () = self.replica_sets.changed() => {}
            () = self.jobs.changed() => {}
            () = self.pods.changed() => {}
            () = self.slices.changed() => {}
        }
    }
}

fn owners(meta: &ObjectMeta) -> impl Iterator<Item = &str> {
    meta.owner_references.iter().flatten().map(|owner| owner.uid.as_str())
}

fn child(kind: Kind, key: &ObjectKey, meta: &ObjectMeta) -> ObjectRef {
    ObjectRef { kind, key: key.clone(), uid: meta.uid.as_deref().unwrap_or_default().into() }
}

/// Applies one change of a store of owned objects. True when a relation changed: most
/// changes of an object are a new status and leave its owners as they are.
fn follow_owned<T: Object>(children: &mut Links<Arc<str>, ObjectRef>, kind: Kind, change: Change<'_, T>) -> bool {
    let (key, old, new) = match change {
        Change::Added(key, new) => (key, None, Some(new.meta())),
        Change::Removed(key, old) => (key, Some(old.meta()), None),
        Change::Updated { key, old, new } => (key, Some(old.meta()), Some(new.meta())),
    };
    if let (Some(old), Some(new)) = (old, new)
        && old.uid == new.uid
        && owners(old).eq(owners(new))
    {
        return false;
    }
    let mut changed = false;
    if let Some(old) = old {
        let gone = child(kind, key, old);
        owners(old).for_each(|owner| changed |= children.remove(owner, &gone));
    }
    if let Some(new) = new {
        owners(new).for_each(|owner| changed |= children.add(owner.into(), child(kind, key, new)));
    }
    changed
}

/// The Service of an EndpointSlice. It is in the namespace of the slice.
fn service_of(key: &ObjectKey, slice: &EndpointSlice) -> Option<ObjectKey> {
    let name = slice.metadata.labels.as_ref()?.get(SERVICE_NAME)?;
    Some(ObjectKey { namespace: Arc::clone(&key.namespace), name: name.as_str().into() })
}

/// Applies one change of the store of the EndpointSlices. Each change is a change of the
/// index: the pods behind a Service come from the endpoints of its slices.
fn follow_slice(slices: &mut Links<ObjectKey, ObjectKey>, change: Change<'_, EndpointSlice>) -> bool {
    let (key, old, new) = match change {
        Change::Added(key, new) => (key, None, service_of(key, new)),
        Change::Removed(key, old) => (key, service_of(key, old), None),
        Change::Updated { key, old, new } => (key, service_of(key, old), service_of(key, new)),
    };
    if old != new {
        if let Some(old) = old {
            slices.remove(&old, key);
        }
        if let Some(new) = new {
            slices.add(new, key.clone());
        }
    }
    true
}
