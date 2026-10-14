//! The stores on demand (plan section 9.5): the full objects of a kind of tier B or tier C
//! while a view needs them.
//!
//! A [`Demand`] is one holder of a store. The first holder starts the store, and the store
//! stops `linger` after the last holder went. A view that the user closes and opens again
//! in that time finds its objects current, with no new list.
//!
//! - `idle`: the holders of each store, and the end of the time of a store with none.

mod idle;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName};
use tokio::sync::Notify;
use tokio::time::{Instant, sleep_until};

use super::cluster::StoreSettings;
use super::snapshot::Snapshot;
use super::starter::{Form, Running, Starter};
use crate::error::CoreError;
use crate::kinds::{Describe, Kind};
use crate::published::{Held, Reader};
use idle::Entries;

/// A reader of the full objects of one kind, for a view. The store stays on while a
/// holder of it lives. A clone is one more holder.
pub struct Demand<K> {
    reader: Reader<Snapshot<K>>,
    /// `None` for a store that is always on.
    hold: Option<Hold>,
}

impl<K: Send + Sync + 'static> Demand<K> {
    /// The holder of a store that is always on.
    pub(super) fn always(reader: Reader<Snapshot<K>>) -> Self {
        Self { reader, hold: None }
    }

    /// The objects and the state of the store now.
    pub fn read(&self) -> Held<Snapshot<K>> {
        self.reader.read()
    }

    /// The reader of the store, for a wait for its next snapshot. A reader with no holder
    /// shows the state "stopped" after the end of the store.
    pub fn reader(&self) -> &Reader<Snapshot<K>> {
        &self.reader
    }
}

impl<K> Clone for Demand<K> {
    fn clone(&self) -> Self {
        Self { reader: self.reader.clone(), hold: self.hold.as_ref().map(Hold::again) }
    }
}

/// One holder of a store on demand. Its drop tells the stores that it went.
struct Hold {
    /// A holder does not keep the stores of a cluster that stopped.
    shared: Weak<Shared>,
    kind: Kind,
}

impl Hold {
    fn again(&self) -> Self {
        if let Some(shared) = self.shared.upgrade() {
            shared.entries().hold(self.kind);
        }
        Self { shared: self.shared.clone(), kind: self.kind }
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        let last = shared.entries().release(self.kind);
        if last {
            shared.released.notify_one();
        }
    }
}

/// The stores on demand of one cluster. Each one stops when this value drops.
pub(super) struct Demands {
    _idle_stop: TaskHandle,
    shared: Arc<Shared>,
}

struct Shared {
    starter: Starter,
    /// The time that a store stays on after its last holder went.
    linger: Duration,
    entries: Mutex<Entries<Running>>,
    /// Wakes the task that stops each idle store, after the last holder of a store went.
    released: Arc<Notify>,
}

impl Shared {
    /// Each use holds the lock for one small change, never across a wait.
    fn entries(&self) -> MutexGuard<'_, Entries<Running>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Demands {
    /// Starts the task that stops each store whose time is over.
    pub(super) fn start(starter: Starter, linger: Duration) -> Self {
        let released = Arc::new(Notify::new());
        let name = TaskName::scoped("idle stores", starter.cluster().as_str());
        let tasks = starter.tasks.clone();
        let entries = Mutex::new(Entries::default());
        let shared = Arc::new(Shared { starter, linger, entries, released: Arc::clone(&released) });
        let weak = Arc::downgrade(&shared);
        let restart = Restart::Always(Backoff::default());
        let task = tasks.start(name, restart, move || stop_idle(weak.clone(), Arc::clone(&released)));
        Self { _idle_stop: task, shared }
    }

    pub(super) fn settings(&self) -> &StoreSettings {
        &self.shared.starter.parts.settings
    }

    /// A holder of the store of the full objects of `K`. The first holder starts it.
    /// `None` only when the store of the kind holds an other type, which no kind of the
    /// list does.
    pub(super) fn demand<K: Describe>(&self, max_items: Option<usize>) -> Option<Demand<K>> {
        let kind = K::ID;
        let starter = &self.shared.starter;
        let mut entries = self.shared.entries();
        let reader = entries.hold_or_start(kind, || starter.start::<K>(kind, Form::Full, None, max_items)).reader();
        let hold = Hold { shared: Arc::downgrade(&self.shared), kind };
        drop(entries);
        // With no reader, the drop of the holder starts the end of the store.
        Some(Demand { reader: reader?, hold: Some(hold) })
    }
}

impl Drop for Demands {
    /// The stores stop now, also while a holder lives: its reader shows "stopped".
    fn drop(&mut self) {
        let stores = self.shared.entries().take_all();
        drop(stores);
    }
}

/// The body of the task: it stops each store at the end of its time, and waits for the
/// next end or for the next store that loses its last holder.
async fn stop_idle(shared: Weak<Shared>, released: Arc<Notify>) -> Result<(), CoreError> {
    loop {
        let next_release = released.notified();
        tokio::pin!(next_release);
        // The registration comes before the read, so that no release is lost.
        next_release.as_mut().enable();
        let Some(shared) = shared.upgrade() else {
            // The stores of the cluster stopped, and the stop of this task follows.
            return std::future::pending().await;
        };
        let (stopped, next) = shared.entries().take_idle(Instant::now(), shared.linger);
        // A store publishes its last snapshot at its stop, so it stops after the lock.
        drop((stopped, shared));
        match next {
            Some(end) => {
                tokio::select! {
                    () = &mut next_release => {}
                    () = sleep_until(end) => {}
                }
            }
            None => next_release.await,
        }
    }
}
