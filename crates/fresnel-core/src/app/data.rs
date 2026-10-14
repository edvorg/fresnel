//! The data of one connected cluster: its stores and its indexes, and what the views read
//! of them.
//!
//! The core starts the stores with the connections of a cluster and stops them with the
//! disconnect. A view reads a copy of the snapshots; it holds no handle to a store.

use std::any::Any;
use std::collections::BTreeSet;
use std::fmt;
use std::pin::pin;
use std::sync::Arc;

use fresnel_net::WallClock;
use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName};
use futures_util::future::select;
use k8s_openapi::api::core::v1::Pod;

use super::keys::{ClusterKeys, KeysHolder, KeysView};
use super::objects::{Objects, ShownKinds};
use super::repaint::Repaint;
use super::{Inner, watch};
use crate::cluster::{ClusterId, Gateway, Link};
use crate::error::CoreError;
use crate::index::{ClusterIndexes, Index, IndexParts, ObjectEvents, Owners, References};
use crate::keys::Target;
use crate::kinds::{Describe, Kind};
use crate::published::{Changes, Disposal, Held, Reader};
use crate::search::Source;
use crate::store::{ClusterStores, Snapshot, StoreParts, StoreStatus};

/// The objects of one connected cluster as the views read them, at one moment.
#[derive(Clone)]
pub struct ClusterData {
    /// The Pods of the cluster, with the state of their store.
    pub pods: Held<Snapshot<Pod>>,
    /// The Event index. `None` when the settings give the Events no store.
    pub events: Option<Held<Index<ObjectEvents>>>,
    /// The owner index: the objects below a workload, and the pods behind a Service.
    pub owners: Held<Index<Owners>>,
    /// The reference index: the pods that use an object.
    pub references: Held<Index<References>>,
    /// The facts of each store that is always on, in the sequence of the kinds: its state
    /// and its count of objects.
    pub stores: Arc<[StoreStatus]>,
    /// The snapshot of each store that is always on, in the sequence of the kinds, for the
    /// quick search: the full objects of a kind of tier A, the metadata of one of tier B.
    pub sources: Arc<[Source]>,
    /// The full objects of the kinds that the views show (`Command::ShowKinds`).
    pub objects: Objects,
    /// The object of the key view, when it is an object of this cluster (`Command::ShowKeys`).
    pub keys: Option<Arc<KeysView>>,
}

impl ClusterData {
    /// The full objects of a kind: the pods, which the view always has, or a kind that a
    /// view shows. `None` for an other kind, and before the core holds the store of a kind
    /// that a view showed.
    pub fn snapshot<K: Describe>(&self) -> Option<Held<Snapshot<K>>> {
        let pods: &dyn Any = &self.pods;
        pods.downcast_ref().or_else(|| self.objects.get::<K>()).cloned()
    }

    /// The facts of the store of a kind: the store that is always on, or the store of the
    /// full objects of a kind of tier C while a view shows it. `None` for an other kind.
    pub fn store(&self, kind: Kind) -> Option<&StoreStatus> {
        self.stores.iter().find(|status| status.kind == kind).or_else(|| self.objects.status(kind))
    }

    /// The store of each kind that the quick search reads: the full objects of a kind that
    /// a view shows, after their first complete list, or else the store that is always on.
    /// A kind with neither has no source.
    pub fn search_sources(&self) -> Vec<Source> {
        let source = |kind: Kind| {
            let shown = self.objects.source(kind);
            let always = self.sources.iter().find(|source| source.status.kind == kind);
            shown.filter(|shown| shown.status.listed).or(always).or(shown).cloned()
        };
        Kind::ALL.iter().copied().filter_map(source).collect()
    }
}

impl fmt::Debug for ClusterData {
    /// The facts of the stores, with no object: a cluster can have 10,000 pods.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClusterData")
            .field("pods", &self.pods.items.len())
            .field("pod_store", &self.pods.state)
            .field("events", &self.events.as_ref().map(|index| &index.state))
            .field("objects", &self.objects)
            .finish_non_exhaustive()
    }
}

/// What the core runs for one connected cluster: its stores, its indexes and the tasks
/// that follow them. Each part stops when this value drops.
///
/// The sequence of the fields is the sequence of the stop: the indexes go before the
/// stores that they read, and the state watch goes last, because it holds the connections
/// of the cluster open.
pub(super) struct Connected {
    keys: ClusterKeys,
    gateway: Gateway,
    pods: Reader<Snapshot<Pod>>,
    events: Option<Reader<Index<ObjectEvents>>>,
    owners: Reader<Index<Owners>>,
    references: Reader<Index<References>>,
    _data_watch: TaskHandle,
    shown: ShownKinds,
    _indexes: ClusterIndexes,
    stores: ClusterStores,
    _state_watch: TaskHandle,
}

impl Connected {
    /// Starts the stores, the indexes and the tasks of a cluster on its connections, with
    /// a store of the full objects of each kind of `shown`.
    pub(super) fn start(inner: &Inner, id: &ClusterId, link: &Link, shown: &BTreeSet<Kind>) -> Result<Self, CoreError> {
        let (tasks, settings) = (&inner.tasks, &inner.settings.stores);
        let bin = inner.disposal.get_or_init(|| Disposal::start(tasks)).bin();
        let parts = StoreParts { settings: settings.clone(), bin: bin.clone(), clock: WallClock::system() };
        let stores = ClusterStores::start(tasks, link, parts)?;
        let indexes = ClusterIndexes::start(tasks, &stores, IndexParts { bin, restart: settings.restart })?;
        let absent = || CoreError::invalid("the cluster has no store of the kind Pod");
        let pods = stores.full::<Pod>().ok_or_else(absent)?;
        let always = Restart::Always(Backoff::default());
        let data_watch = tasks.start(TaskName::scoped("data watch", id.as_str()), always, {
            let (stores, indexes, repaint) = (stores.notice(), indexes.notice(), inner.repaint.clone());
            move || follow(stores.changes(), indexes.changes(), repaint.clone())
        });
        let state_watch = watch::start(tasks, id, link.net.clone(), inner.journal.clone(), inner.repaint.clone());
        let mut shown_kinds = ShownKinds::default();
        shown_kinds.set(&stores, shown);
        Ok(Self {
            keys: ClusterKeys::default(),
            gateway: link.gateway.clone(),
            pods,
            events: indexes.events(),
            owners: indexes.owners(),
            references: indexes.references(),
            _data_watch: data_watch,
            shown: shown_kinds,
            _indexes: indexes,
            stores,
            _state_watch: state_watch,
        })
    }

    /// Holds the store of the full objects of each kind of `kinds`, and lets the others go.
    pub(super) fn show(&mut self, kinds: &BTreeSet<Kind>) {
        self.shown.set(&self.stores, kinds);
    }

    /// Follows the object of the key view when it is an object of this cluster, and drops
    /// the object of an earlier view. The same object again keeps its holder.
    pub(super) fn show_keys(&mut self, inner: &Inner, target: Option<&Target>) {
        let target = target.filter(|target| &target.cluster == self.stores.cluster());
        let (gateway, stores) = (&self.gateway, &self.stores);
        self.keys.show(target, |target| {
            KeysHolder::start(inner, target.clone(), gateway.clone(), stores.metadata(target.kind.kind()))
        });
    }

    /// The holder of the object of the key view, when it is the object of `target`.
    pub(super) fn keys(&self, target: &Target) -> Option<&KeysHolder> {
        self.keys.holder(target)
    }

    /// A copy of what the views read. The call copies a pointer for each store and index.
    pub(super) fn data(&self) -> ClusterData {
        let sources: Arc<[Source]> = self.stores.sources().into();
        ClusterData {
            pods: self.pods.read(),
            events: self.events.as_ref().map(Reader::read),
            owners: self.owners.read(),
            references: self.references.read(),
            stores: sources.iter().map(|source| source.status.clone()).collect(),
            sources,
            objects: self.shown.read(),
            keys: self.keys.view(),
        }
    }
}

/// Asks the UI for a repaint for each new snapshot of a store and each new value of an
/// index: the sidebar shows the count of each store, and a table the objects of its kind.
/// The notifier of the repaints limits their rate.
async fn follow(mut stores: Changes, mut indexes: Changes, repaint: Repaint) -> Result<(), CoreError> {
    loop {
        repaint.request();
        select(pin!(stores.next()), pin!(indexes.next())).await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, SystemTime};

    use fresnel_net::task::{TaskReport, Tasks};
    use k8s_openapi::api::core::v1::Event;
    use tokio::runtime::Handle;
    use tokio::time::sleep;

    use super::*;
    use crate::index::IndexState;
    use crate::published::{Notice, Publisher};
    use crate::store::StoreState;

    /// Nothing else tells the window that its rows are old: a state of a channel can stay
    /// the same for a long time while the pods of the cluster change.
    #[tokio::test(start_paused = true)]
    async fn each_new_snapshot_and_each_new_index_ask_for_a_repaint_and_nothing_else_does() {
        let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task ended: {report}"));
        let disposal = Disposal::start(&tasks);
        let snapshot = |state| Snapshot::<Pod>::empty(state, SystemTime::UNIX_EPOCH);
        let index = || {
            let events = Snapshot::<Event>::empty(StoreState::Live, SystemTime::UNIX_EPOCH);
            Index::<ObjectEvents>::of_snapshot(&events, IndexState::Following)
        };
        let (of_stores, of_indexes) = (Notice::new(), Notice::new());
        let pods = Publisher::new(snapshot(StoreState::Live), disposal.bin(), of_stores.clone());
        let events = Publisher::new(index(), disposal.bin(), of_indexes.clone());

        let repaint = Repaint::new();
        let wakes = Arc::new(AtomicU32::new(0));
        let count = Arc::clone(&wakes);
        repaint.set_wake(Arc::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
        }));
        let always = Restart::Always(Backoff::default());
        let _notifier = tasks.start(TaskName::new("repaint notifier"), always, {
            let repaint = repaint.clone();
            move || repaint.clone().run(Duration::from_millis(1))
        });
        let _watch = tasks.start(TaskName::new("data watch"), always, {
            move || follow(of_stores.changes(), of_indexes.changes(), repaint.clone())
        });
        let wakes_after = |pause: Duration| {
            let wakes = Arc::clone(&wakes);
            async move {
                sleep(pause).await;
                wakes.load(Ordering::Relaxed)
            }
        };
        let moment = Duration::from_millis(10);
        assert_eq!(wakes_after(moment).await, 1, "the first values");

        pods.publish(snapshot(StoreState::Stopped));
        assert_eq!(wakes_after(moment).await, 2, "a new snapshot of a store");
        events.publish(index());
        assert_eq!(wakes_after(moment).await, 3, "a new value of an index");
        assert_eq!(wakes_after(Duration::from_secs(3600)).await, 3, "no new value, no repaint");
    }
}
