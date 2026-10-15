//! What the sampler reads of the stores and the owner index at each poll: the objects that
//! exist, the pods of each workload and the requests on each node.
//!
//! A poll reads the stores once and keeps plain data, so the sums of a poll are of one
//! moment, and a unit test builds a world with no store.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Namespace, Node, Pod};

use super::history::SeriesKey;
use super::resources::{self, Amount};
use crate::error::CoreError;
use crate::index::{ClusterIndexes, Index, Owners};
use crate::kinds::{Describe, Kind, Object};
use crate::published::Reader;
use crate::store::{ClusterStores, ObjectKey, Snapshot};

/// The requests on one node at the time of a poll.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeLoad {
    /// The sum of the requests of the pods on the node that are not done: a pod in the
    /// phase `Succeeded` or `Failed` holds nothing (research 11 section 6).
    pub requests: Amount,
    /// The count of those pods.
    pub pods: u32,
}

/// The objects of one store at a poll, and if the store had its first complete list.
#[derive(Clone, Debug, Default)]
pub(crate) struct Known<T> {
    pub(crate) listed: bool,
    pub(crate) items: T,
}

impl<T> Known<T> {
    /// True or false when the store knows; `None` for an object that a store with no
    /// complete list does not have.
    fn tells(&self, has: impl FnOnce(&T) -> bool) -> Option<bool> {
        if has(&self.items) { Some(true) } else { self.listed.then_some(false) }
    }
}

/// The stores at the time of one poll.
#[derive(Clone, Debug, Default)]
pub(crate) struct World {
    /// Each pod, with true for a pod that is not done.
    pub(crate) pods: Known<BTreeMap<ObjectKey, bool>>,
    pub(crate) nodes: Known<BTreeSet<Arc<str>>>,
    pub(crate) namespaces: Known<BTreeSet<Arc<str>>>,
    /// Each workload of each kind with the pods that it owns, from the owner index.
    pub(crate) workloads: BTreeMap<Kind, Known<BTreeMap<ObjectKey, Vec<ObjectKey>>>>,
    pub(crate) loads: BTreeMap<Arc<str>, NodeLoad>,
}

impl World {
    /// True when the object of a series is in its store. `None` when its store has no
    /// complete list and does not have it.
    pub(crate) fn exists(&self, key: &SeriesKey) -> Option<bool> {
        match key {
            SeriesKey::Container { pod, .. } | SeriesKey::Pod(pod) => self.pods.tells(|pods| pods.contains_key(pod)),
            SeriesKey::Workload(kind, key) => self.workloads.get(kind)?.tells(|of| of.contains_key(key)),
            SeriesKey::Node(name) => self.nodes.tells(|nodes| nodes.contains(name)),
            SeriesKey::Namespace(name) => self.namespaces.tells(|names| names.contains(name)),
            SeriesKey::Cluster => Some(true),
        }
    }

    /// True for a pod that the store has and that is not done: a sum expects its value.
    pub(crate) fn runs(&self, pod: &ObjectKey) -> bool {
        self.pods.items.get(pod).copied().unwrap_or_default()
    }
}

/// The objects of one workload kind: identity and UID, from a store of full objects or of
/// metadata.
trait Workloads: Send + Sync {
    fn kind(&self) -> Kind;

    /// Each object with its UID, and if the store had its first complete list.
    fn read(&self) -> Known<Vec<(ObjectKey, Arc<str>)>>;
}

struct Store<T> {
    kind: Kind,
    reader: Reader<Snapshot<T>>,
}

impl<T: Object> Workloads for Store<T> {
    fn kind(&self) -> Kind {
        self.kind
    }

    fn read(&self) -> Known<Vec<(ObjectKey, Arc<str>)>> {
        let snapshot = self.reader.read();
        let uid = |object: &Arc<T>| object.meta().uid.as_deref().map(Arc::from);
        let items = snapshot.items.iter().filter_map(|(key, object)| Some((key.clone(), uid(object)?))).collect();
        Known { listed: snapshot.listed, items }
    }
}

/// The readers of the stores and the owner index that a sampler needs.
pub struct Inputs {
    pods: Reader<Snapshot<Pod>>,
    nodes: Reader<Snapshot<Node>>,
    namespaces: Reader<Snapshot<Namespace>>,
    owners: Reader<Index<Owners>>,
    workloads: Vec<Box<dyn Workloads>>,
}

impl Inputs {
    /// The readers of the stores and the indexes of one cluster. An error for stores that
    /// do not have each kind of tier A.
    pub fn of(stores: &ClusterStores, indexes: &ClusterIndexes) -> Result<Self, CoreError> {
        fn full<K: Describe>(stores: &ClusterStores) -> Result<Reader<Snapshot<K>>, CoreError> {
            let absent = || CoreError::invalid(format!("the cluster has no store of the kind {}", K::ID.info().name));
            stores.full::<K>().ok_or_else(absent)
        }
        fn workload<K: Describe>(stores: &ClusterStores) -> Result<Box<dyn Workloads>, CoreError> {
            Ok(Box::new(Store { kind: K::ID, reader: full::<K>(stores)? }))
        }
        let controllers = stores.metadata(Kind::ReplicationController);
        let controllers = controllers
            .ok_or_else(|| CoreError::invalid("the cluster has no store of the kind ReplicationController"))?;
        let workloads = vec![
            workload::<Deployment>(stores)?,
            workload::<StatefulSet>(stores)?,
            workload::<DaemonSet>(stores)?,
            workload::<ReplicaSet>(stores)?,
            Box::new(Store { kind: Kind::ReplicationController, reader: controllers }),
            workload::<Job>(stores)?,
            workload::<CronJob>(stores)?,
        ];
        Ok(Self {
            pods: full(stores)?,
            nodes: full(stores)?,
            namespaces: full(stores)?,
            owners: indexes.owners(),
            workloads,
        })
    }

    /// The world of now.
    pub(crate) fn read(&self) -> World {
        let pods = self.pods.read();
        let mut loads: BTreeMap<Arc<str>, NodeLoad> = BTreeMap::new();
        let mut states = BTreeMap::new();
        for (key, pod) in pods.items.iter() {
            let runs = !resources::is_done(pod);
            states.insert(key.clone(), runs);
            let node = pod.spec.as_ref().and_then(|spec| spec.node_name.as_deref());
            if let Some(node) = node.filter(|_| runs) {
                let load = loads.entry(node.into()).or_default();
                load.requests = load.requests + resources::of_pod(pod).requests;
                load.pods += 1;
            }
        }
        let owners = self.owners.read();
        let workloads = self.workloads.iter().map(|workloads| {
            let Known { listed, items } = workloads.read();
            let kind = workloads.kind();
            let pods_of = |uid: &str| owners.pods_of(kind, uid).found.into_iter().map(|pod| pod.key).collect();
            let items = items.into_iter().map(|(key, uid)| (key, pods_of(&uid))).collect();
            (kind, Known { listed, items })
        });
        World {
            pods: Known { listed: pods.listed, items: states },
            nodes: names(&self.nodes.read()),
            namespaces: names(&self.namespaces.read()),
            workloads: workloads.collect(),
            loads,
        }
    }
}

/// The names of the objects of a store of a kind with no namespace.
fn names<T>(snapshot: &Snapshot<T>) -> Known<BTreeSet<Arc<str>>> {
    Known { listed: snapshot.listed, items: snapshot.items.iter().map(|(key, _)| Arc::clone(&key.name)).collect() }
}

#[cfg(test)]
pub(super) mod tests;
