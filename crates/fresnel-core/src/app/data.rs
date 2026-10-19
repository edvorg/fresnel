//! The data of one connected cluster: its stores, its indexes, its metrics and the version
//! of its server, and what the views read of them.
//!
//! The core starts the stores with the connections of a cluster and stops them with the
//! disconnect. A view reads a copy of the snapshots; it holds no handle to a store.

use std::any::Any;
use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use fresnel_net::WallClock;
use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName};
use futures_util::future::select_all;
use k8s_openapi::api::core::v1::Pod;

use super::access::{AccessView, ClusterAccess};
use super::jobs::ClusterParts;
use super::keys::{ClusterKeys, KeysHolder, KeysView};
use super::logs::ClusterLogs;
use super::objects::{Objects, ShownKinds};
use super::repaint::Repaint;
use super::{Inner, watch};
use crate::cluster::{ClusterId, ClusterVersion, Gateway, Link, ServerVersion};
use crate::error::CoreError;
use crate::forward::Feed;
use crate::index::{ClusterIndexes, Index, IndexParts, ObjectEvents, Owners, References};
use crate::keys::Target;
use crate::kinds::{Describe, Kind, Witness};
use crate::logs::{LogRequest, LogSave, LogView, SaveParts, SaveView, SessionParts};
use crate::metrics::{Inputs, Metrics, Sampler, SamplerParts};
use crate::ops::{Cancel, Parts};
use crate::published::{Changes, Disposal, Held, Notice, Reader};
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
    /// quick search and the counts of a namespace: the full objects of a kind of tier A, the
    /// metadata of one of tier B.
    pub sources: Arc<[Source]>,
    /// The full objects of the kinds that the views show (`Command::ShowKinds`).
    pub objects: Objects,
    /// The object of the key view, when it is an object of this cluster (`Command::ShowKeys`).
    pub keys: Option<Arc<KeysView>>,
    /// The permissions of the user (`Command::ReadRules`, `Command::CheckAccess`).
    pub access: Arc<AccessView>,
    /// The metrics: the state of each poll store, the history and the requests on each node.
    pub metrics: Held<Metrics>,
    /// The version of the API server, for the cluster overview.
    pub version: Held<ServerVersion>,
    /// The log views of objects of this cluster (`Command::ShowLogs`).
    pub logs: Arc<[Held<LogView>]>,
    /// The saves of the log views of this cluster, the oldest first (`Command::SaveLog`).
    pub saves: Arc<[Held<SaveView>]>,
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

    /// The stores that tell the state of the problem of a warning Event at the moment `at`:
    /// the pods and each other store of full objects that is always on.
    pub fn witness(&self, at: SystemTime) -> Witness {
        Witness::new(std::iter::once(Source::full(self.pods.clone())).chain(self.sources.iter().cloned()), at)
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
            .field("logs", &self.logs.len())
            .field("metrics", &format_args!("pods {}, nodes {}", self.metrics.pods.state, self.metrics.nodes.state))
            .finish_non_exhaustive()
    }
}

/// What the core runs for one connected cluster: its stores, its indexes and the tasks
/// that follow them. Each part stops when this value drops.
///
/// The sequence of the fields is the sequence of the stop: the sampler and the indexes go
/// before the stores that they read, and the state watch goes last, because it holds the
/// connections of the cluster open.
pub(super) struct Connected {
    /// What the forwards of the cluster read. `None` for a cluster with no store of pods.
    feed: Option<Arc<Feed>>,
    logs: ClusterLogs,
    keys: ClusterKeys,
    access: ClusterAccess,
    gateway: Gateway,
    pods: Reader<Snapshot<Pod>>,
    events: Option<Reader<Index<ObjectEvents>>>,
    owners: Reader<Index<Owners>>,
    references: Reader<Index<References>>,
    metrics: Reader<Metrics>,
    version: Reader<ServerVersion>,
    _data_watch: TaskHandle,
    _version: ClusterVersion,
    shown: ShownKinds,
    _sampler: Sampler,
    _indexes: ClusterIndexes,
    stores: ClusterStores,
    _state_watch: TaskHandle,
}

impl Connected {
    /// Starts the stores, the indexes and the tasks of a cluster on its connections, with
    /// a store of the full objects of each kind of `shown`.
    pub(super) fn start(inner: &Inner, id: &ClusterId, link: &Link, shown: &BTreeSet<Kind>) -> Result<Self, CoreError> {
        let (tasks, settings) = (&inner.tasks, &inner.settings().stores);
        let bin = inner.disposal.get_or_init(|| Disposal::start(tasks)).bin();
        let parts = StoreParts { settings: settings.clone(), bin: bin.clone(), clock: WallClock::system() };
        let stores = ClusterStores::start(tasks, link, parts)?;
        let indexes =
            ClusterIndexes::start(tasks, &stores, IndexParts { bin: bin.clone(), restart: settings.restart })?;
        let absent = || CoreError::invalid("the cluster has no store of the kind Pod");
        let pods = stores.full::<Pod>().ok_or_else(absent)?;
        let notice = Notice::new();
        let settings = inner.settings().metrics.clone();
        let clock = WallClock::system();
        let parts =
            SamplerParts { settings, bin: bin.clone(), notice: notice.clone(), clock, signal: link.net.signal() };
        let sampler = Sampler::start(tasks, link.gateway.clone(), Inputs::of(&stores, &indexes)?, parts);
        let version = ClusterVersion::start(tasks, link.gateway.clone(), bin.clone(), notice.clone());
        let always = Restart::Always(Backoff::default());
        let logs = Notice::new();
        let data_watch = tasks.start(TaskName::scoped("data watch", id.as_str()), always, {
            let notices = [stores.notice(), indexes.notice(), notice, logs.clone()];
            let repaint = inner.repaint.clone();
            move || follow(notices.iter().map(Notice::changes).collect(), repaint.clone())
        });
        let save_parts = SaveParts {
            tasks: tasks.clone(),
            folder: inner.downloads.clone(),
            clock: WallClock::system(),
            bin: bin.clone(),
            notice: logs.clone(),
        };
        let log_parts = SessionParts {
            tasks: tasks.clone(),
            pods: pods.clone(),
            owners: indexes.owners(),
            signal: link.net.signal(),
            clock: WallClock::system(),
            settings: inner.settings().logs.clone(),
            restart: inner.settings().stores.restart,
            bin,
            notice: logs,
        };
        let state_watch = watch::start(tasks, id, link.net.clone(), inner.journal.clone(), inner.repaint.clone());
        let mut shown_kinds = ShownKinds::default();
        shown_kinds.set(&stores, shown);
        Ok(Self {
            feed: Feed::new(link, &stores, &indexes).map(Arc::new),
            logs: ClusterLogs::new(log_parts, link.gateway.clone(), save_parts),
            keys: ClusterKeys::default(),
            access: ClusterAccess::start(tasks, id, link.gateway.clone(), inner.repaint.clone()),
            gateway: link.gateway.clone(),
            pods,
            events: indexes.events(),
            owners: indexes.owners(),
            references: indexes.references(),
            metrics: sampler.reader(),
            version: version.reader(),
            _data_watch: data_watch,
            _version: version,
            shown: shown_kinds,
            _sampler: sampler,
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

    /// Runs the log views of objects of this cluster, and stops the others.
    pub(super) fn show_logs(&mut self, requests: &[LogRequest]) {
        self.logs.show(requests);
    }

    /// Starts a save of a log view of this cluster.
    pub(super) fn save_log(&mut self, save: LogSave) {
        self.logs.save(save);
    }

    pub(super) fn cancel_save(&self, number: u64) {
        self.logs.cancel_save(number);
    }

    /// The holder of the object of the key view, when it is the object of `target`.
    pub(super) fn keys(&self, target: &Target) -> Option<&KeysHolder> {
        self.keys.holder(target)
    }

    /// The gateway and the pods of the cluster, for the session of a terminal.
    pub(super) fn terminal_parts(&self) -> (Gateway, Reader<Snapshot<Pod>>) {
        (self.gateway.clone(), self.pods.clone())
    }

    /// The snapshot of each store on demand and the readers of the lanes, for the memory
    /// account.
    pub(super) fn memory(&self) -> (Vec<Source>, usize) {
        (self.stores.demand_sources(), self.stores.lane_readers())
    }

    /// What the forwards of the cluster read.
    pub(super) fn feed(&self) -> Option<Arc<Feed>> {
        self.feed.clone()
    }

    /// The permissions of the user in the cluster.
    pub(super) fn access(&self) -> &ClusterAccess {
        &self.access
    }

    /// What a job on an object of the cluster gets: the gateway, the stores that its steps
    /// follow, its cancel and the permissions.
    pub(super) fn job_parts(&self, cancel: Cancel, downloads: PathBuf) -> ClusterParts {
        let stores = &self.stores;
        let parts = Parts {
            gateway: self.gateway.clone(),
            deployments: stores.full(),
            stateful_sets: stores.full(),
            daemon_sets: stores.full(),
            jobs: stores.full(),
            pods: Some(self.pods.clone()),
            downloads,
            cancel,
            after: None,
        };
        (parts, self.access.handle())
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
            access: self.access.view(),
            metrics: self.metrics.read(),
            version: self.version.read(),
            logs: self.logs.views(),
            saves: self.logs.saves(),
        }
    }
}

/// Asks the UI for a repaint for each new snapshot of a store, each new value of an index
/// each poll of the metrics and each new value of a log view: the sidebar shows the count of
/// each store, and a table the objects of its kind. The notifier limits the rate.
pub(super) async fn follow(mut notices: Vec<Changes>, repaint: Repaint) -> Result<(), CoreError> {
    loop {
        repaint.request();
        select_all(notices.iter_mut().map(|changes| Box::pin(changes.next()))).await;
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
    use crate::published::Publisher;
    use crate::store::StoreState;

    /// Nothing else tells the window that its rows are old: a state of a channel can stay
    /// the same for a long time while the pods of the cluster change.
    #[tokio::test(start_paused = true)]
    async fn each_new_snapshot_index_and_poll_asks_for_a_repaint_and_nothing_else_does() {
        let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task ended: {report}"));
        let disposal = Disposal::start(&tasks);
        let snapshot = |state| Snapshot::<Pod>::empty(state, SystemTime::UNIX_EPOCH);
        let index = || {
            let events = Snapshot::<Event>::empty(StoreState::Live, SystemTime::UNIX_EPOCH);
            Index::<ObjectEvents>::of_snapshot(&events, IndexState::Following)
        };
        let (of_stores, of_indexes, of_metrics) = (Notice::new(), Notice::new(), Notice::new());
        let pods = Publisher::new(snapshot(StoreState::Live), disposal.bin(), of_stores.clone());
        let events = Publisher::new(index(), disposal.bin(), of_indexes.clone());
        let polls = || Metrics::waiting(ClusterId::new("ams"), Duration::from_secs(15), SystemTime::UNIX_EPOCH);
        let metrics = Publisher::new(polls(), disposal.bin(), of_metrics.clone());

        let repaint = Repaint::new();
        repaint.set_limit(1000);
        let wakes = Arc::new(AtomicU32::new(0));
        let count = Arc::clone(&wakes);
        repaint.set_wake(Arc::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
        }));
        let always = Restart::Always(Backoff::default());
        let _notifier = tasks.start(TaskName::new("repaint notifier"), always, {
            let repaint = repaint.clone();
            move || repaint.clone().run()
        });
        let _watch = tasks.start(TaskName::new("data watch"), always, {
            move || follow(vec![of_stores.changes(), of_indexes.changes(), of_metrics.changes()], repaint.clone())
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
        metrics.publish(polls());
        assert_eq!(wakes_after(moment).await, 4, "a poll of the metrics");
        assert_eq!(wakes_after(Duration::from_secs(3600)).await, 4, "no new value, no repaint");
    }
}
