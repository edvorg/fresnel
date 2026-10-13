//! The data of one connected cluster: its stores and its indexes, and what the views read
//! of them.
//!
//! The core starts the stores with the connections of a cluster and stops them with the
//! disconnect. A view reads a copy of the snapshots; it holds no handle to a store.

use std::fmt;
use std::pin::pin;

use fresnel_net::WallClock;
use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName};
use futures_util::future::select;
use k8s_openapi::api::core::v1::Pod;

use super::repaint::Repaint;
use super::{Inner, watch};
use crate::cluster::{ClusterId, Link};
use crate::error::CoreError;
use crate::index::{ClusterIndexes, Index, IndexParts, ObjectEvents};
use crate::published::{Disposal, Held, Reader};
use crate::store::{ClusterStores, Snapshot, StoreParts};

/// The objects of one connected cluster as the views read them, at one moment.
#[derive(Clone)]
pub struct ClusterData {
    /// The Pods of the cluster, with the state of their store.
    pub pods: Held<Snapshot<Pod>>,
    /// The Event index. `None` when the settings give the Events no store.
    pub events: Option<Held<Index<ObjectEvents>>>,
}

impl fmt::Debug for ClusterData {
    /// The facts of the stores, with no object: a cluster can have 10,000 pods.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClusterData")
            .field("pods", &self.pods.items.len())
            .field("pod_store", &self.pods.state)
            .field("events", &self.events.as_ref().map(|index| &index.state))
            .finish()
    }
}

/// What the core runs for one connected cluster: its stores, its indexes and the tasks
/// that follow them. Each part stops when this value drops.
///
/// The sequence of the fields is the sequence of the stop: the indexes go before the
/// stores that they read, and the state watch goes last, because it holds the connections
/// of the cluster open.
pub(super) struct Connected {
    pods: Reader<Snapshot<Pod>>,
    events: Option<Reader<Index<ObjectEvents>>>,
    _data_watch: TaskHandle,
    _indexes: ClusterIndexes,
    _stores: ClusterStores,
    _state_watch: TaskHandle,
}

impl Connected {
    /// Starts the stores, the indexes and the tasks of a cluster on its connections.
    pub(super) fn start(inner: &Inner, id: &ClusterId, link: &Link) -> Result<Self, CoreError> {
        let (tasks, settings) = (&inner.tasks, &inner.settings.stores);
        let bin = inner.disposal.get_or_init(|| Disposal::start(tasks)).bin();
        let parts = StoreParts { settings: settings.clone(), bin: bin.clone(), clock: WallClock::system() };
        let stores = ClusterStores::start(tasks, link, parts)?;
        let indexes = ClusterIndexes::start(tasks, &stores, IndexParts { bin, restart: settings.restart })?;
        let absent = || CoreError::invalid("the cluster has no store of the kind Pod");
        let pods = stores.full::<Pod>().ok_or_else(absent)?;
        let events = indexes.events();
        let always = Restart::Always(Backoff::default());
        let data_watch = tasks.start(TaskName::scoped("data watch", id.as_str()), always, {
            let (pods, events, repaint) = (pods.clone(), events.clone(), inner.repaint.clone());
            move || follow(pods.clone(), events.clone(), repaint.clone())
        });
        let state_watch = watch::start(tasks, id, link.net.clone(), inner.journal.clone(), inner.repaint.clone());
        Ok(Self {
            pods,
            events,
            _data_watch: data_watch,
            _indexes: indexes,
            _stores: stores,
            _state_watch: state_watch,
        })
    }

    /// A copy of what the views read. The call copies two pointers.
    pub(super) fn data(&self) -> ClusterData {
        ClusterData { pods: self.pods.read(), events: self.events.as_ref().map(Reader::read) }
    }
}

/// Asks the UI for a repaint for each new value that a view reads: the snapshot of the
/// Pods and the Event index. A store that no view reads asks for none.
async fn follow(
    mut pods: Reader<Snapshot<Pod>>,
    mut events: Option<Reader<Index<ObjectEvents>>>,
    repaint: Repaint,
) -> Result<(), CoreError> {
    loop {
        repaint.request();
        match &mut events {
            Some(events) => {
                select(pin!(pods.changed()), pin!(events.changed())).await;
            }
            None => pods.changed().await,
        }
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
        let pods = Publisher::new(snapshot(StoreState::Live), disposal.bin(), Notice::new());
        let events = Publisher::new(index(), disposal.bin(), Notice::new());

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
            let (pods, events) = (pods.reader(), Some(events.reader()));
            move || follow(pods.clone(), events.clone(), repaint.clone())
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
        assert_eq!(wakes_after(moment).await, 2, "a new snapshot of the pods");
        events.publish(index());
        assert_eq!(wakes_after(moment).await, 3, "a new Event index");
        assert_eq!(wakes_after(Duration::from_secs(3600)).await, 3, "no new value, no repaint");
    }
}
