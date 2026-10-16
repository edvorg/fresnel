//! What a connected cluster gives its forwards: the gateway, the connection layer, and the
//! stores and the owner index that the target reads.
//!
//! A forward outlives the connections of its cluster: its listener stays open while the
//! cluster connects again, and the core gives it the feed of each new connect.

use std::sync::Arc;

use k8s_openapi::api::core::v1::{Pod, Service};

use super::resolve::Owner;
use crate::cluster::{Gateway, Link};
use crate::index::{ClusterIndexes, Index, Owners};
use crate::kinds::{Describe, Kind, Object, Tier, Visit};
use crate::published::{Changes, Notice, Reader};
use crate::store::{ClusterStores, ObjectKey, Snapshot, StoreState};

/// The facts of one store that the target read, for the age of the data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StoreFacts {
    pub(super) kind: Kind,
    pub(super) listed: bool,
    pub(super) state: StoreState,
    pub(super) since: std::time::SystemTime,
}

impl StoreFacts {
    pub(super) fn of<T>(kind: Kind, snapshot: &Snapshot<T>) -> Self {
        Self { kind, listed: snapshot.listed, state: snapshot.state.clone(), since: snapshot.since }
    }
}

/// The store of the object of a workload rule, with no type of its objects.
trait OwnerStore: Send + Sync {
    /// The UID of the object with this key, and the facts of the store.
    fn owner(&self, key: &ObjectKey) -> (Owner, StoreFacts);
}

impl<T: Object> OwnerStore for (Kind, Reader<Snapshot<T>>) {
    fn owner(&self, key: &ObjectKey) -> (Owner, StoreFacts) {
        let snapshot = self.1.read();
        let owner = match (snapshot.listed, snapshot.items.get(key)) {
            (false, _) => Owner::NotLoaded,
            (true, None) => Owner::Gone,
            (true, Some(object)) => object.meta().uid.as_deref().map_or(Owner::Gone, |uid| Owner::Found(uid.into())),
        };
        (owner, StoreFacts::of(self.0, &snapshot))
    }
}

/// The store that is always on of a kind: the full objects of tier A, the metadata of tier B.
struct OwnerStoreOf<'a>(&'a ClusterStores);

impl Visit for OwnerStoreOf<'_> {
    type Output = Option<Arc<dyn OwnerStore>>;

    fn visit<K: Describe>(self) -> Self::Output {
        match K::TIER {
            Tier::A => self.0.full::<K>().map(|reader| Arc::new((K::ID, reader)) as Arc<dyn OwnerStore>),
            Tier::B => self.0.metadata(K::ID).map(|reader| Arc::new((K::ID, reader)) as Arc<dyn OwnerStore>),
            Tier::C => None,
        }
    }
}

/// What a forward reads of its connected cluster. A clone shares the readers.
#[derive(Clone)]
pub struct Feed {
    pub(super) gateway: Gateway,
    pub(super) net: fresnel_net::Cluster,
    pub(super) pods: Reader<Snapshot<Pod>>,
    pub(super) services: Reader<Snapshot<Service>>,
    pub(super) owners: Reader<Index<Owners>>,
    workloads: Arc<[(Kind, Arc<dyn OwnerStore>)]>,
    stores: Notice,
    indexes: Notice,
}

impl Feed {
    /// The feed of a cluster on its stores. `None` when the cluster has no store of the
    /// pods or of the Services: then no forward can find a target.
    pub(crate) fn new(link: &Link, stores: &ClusterStores, indexes: &ClusterIndexes) -> Option<Self> {
        let workloads = super::rule::TARGETS
            .iter()
            .filter(|kind| !matches!(kind, Kind::Pod | Kind::Service))
            .filter_map(|kind| Some((*kind, kind.visit(OwnerStoreOf(stores))?)))
            .collect();
        Some(Self {
            gateway: link.gateway.clone(),
            net: link.net.clone(),
            pods: stores.full()?,
            services: stores.full()?,
            owners: indexes.owners(),
            workloads,
            stores: stores.notice(),
            indexes: indexes.notice(),
        })
    }

    /// The object of a workload rule in its store. `None` for an other kind.
    pub(super) fn owner(&self, kind: Kind, key: &ObjectKey) -> Option<(Owner, StoreFacts)> {
        let (_, store) = self.workloads.iter().find(|(of, _)| *of == kind)?;
        Some(store.owner(key))
    }

    /// The notices of a new snapshot of a store, of a new index and of a new state of the
    /// connections: each one can change the target or the path.
    pub(super) fn changes(&self) -> FeedChanges {
        FeedChanges { stores: self.stores.changes(), indexes: self.indexes.changes(), net: self.net.changes() }
    }
}

/// Waits for a change of the stores, the indexes or the connections of a cluster.
pub(super) struct FeedChanges {
    stores: Changes,
    indexes: Changes,
    net: fresnel_net::Changes,
}

impl FeedChanges {
    /// Waits for the next change. Many changes before a wait count as one.
    pub(super) async fn next(&mut self) {
        tokio::select! {
            () = self.stores.next() => {}
            () = self.indexes.next() => {}
            () = self.net.next() => {}
        }
    }
}

impl std::fmt::Debug for Feed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Feed").field("cluster", self.gateway.cluster()).finish_non_exhaustive()
    }
}
