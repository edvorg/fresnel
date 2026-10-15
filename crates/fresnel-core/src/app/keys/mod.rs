//! The object of the key view (plan section 12.2): one Secret or ConfigMap that the core
//! reads while its view is open, and the saves of its edit.
//!
//! The values of a Secret are in no store. They live in this holder and in the copies of the
//! view, so they go away with the view. The holder is a part of the connected cluster of the
//! object, so it goes with the connections of that cluster too. A new version of the object
//! in the metadata store of its kind, which is always on, gives a new read. A save runs in
//! the task of the reads, so a read and a save never pass each other.
//!
//! - `follow`: the task of one holder.

mod follow;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName};
use tokio::sync::Notify;

use super::{Inner, Phase};
use crate::cluster::Gateway;
use crate::error::CoreError;
use crate::journal::Journal;
use crate::keys::{KeyObject, Save, Target, WriteState};
use crate::kinds::Meta;
use crate::published::Reader;
use crate::store::Snapshot;
use follow::{Parts, Slot, change, follow, lock, pending};

impl Inner {
    /// Follows the object of the key view in its cluster, also after a new connect of the
    /// cluster, and drops the object of the view before.
    pub(super) fn show_keys(&self, target: Option<Target>) {
        let mut state = self.state();
        if state.phase == Phase::Stopped {
            return;
        }
        for connected in state.connected.values_mut() {
            connected.show_keys(self, target.as_ref());
        }
        state.keys = target;
        drop(state);
        self.repaint.request();
    }

    /// Gives a save to the holder of its object. With no holder, the key view of the object
    /// is closed or its cluster is not connected: the save does not go.
    pub(super) fn save_keys(&self, save: Save) {
        let mut state = self.state();
        state.saves += 1;
        let number = state.saves;
        let target = save.target.clone();
        // A read-only cluster gets no write, also from a view that did not close its gate.
        if let Some(refusal) = state.config.refusal(&target.cluster) {
            drop(state);
            self.journal.warning(target.cluster.as_str(), format!("{} did not go: {refusal}", save_of(&target)));
            self.repaint.request();
            return;
        }
        let holder = state.connected.get(&target.cluster).and_then(|connected| connected.keys(&target));
        let Some(holder) = holder else {
            drop(state);
            let text = format!("{} did not go: its key view is not open", save_of(&target));
            self.journal.warning(target.cluster.as_str(), text);
            self.repaint.request();
            return;
        };
        holder.save(number, save);
        drop(state);
        self.repaint.request();
    }
}

/// "the save of Secret shop/db": the start of a journal entry about a save. It names the
/// object and never a value.
fn save_of(target: &Target) -> String {
    let (kind, key) = (target.kind.kind().info().name, &target.key);
    format!("the save of {kind} {}/{}", key.namespace, key.name)
}

/// What the key view sees of its object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeysView {
    /// The object that the view asked for.
    pub target: Target,
    /// The last good read or the answer of the last save. It stays through a later read
    /// that fails (design rule 5). `None` before the first good read.
    pub object: Option<Arc<KeyObject>>,
    /// How far the read is.
    pub read: ReadState,
    /// The last save of the view.
    pub save: Option<SaveJob>,
}

/// How far the read of the object of a key view is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadState {
    /// The request is on its way.
    Reading,
    /// The object is the read of the server.
    Read,
    /// The last read failed. The object of an earlier read stays.
    Failed(CoreError),
}

/// One save of the key view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveJob {
    /// A number that grows with each save of the core.
    pub number: u64,
    /// What the view asked for.
    pub save: Save,
    /// How far the save is.
    pub state: WriteState,
}

/// The key view of one connected cluster: the holder of the object that the view shows,
/// and the holders of earlier objects with a save in progress. The stop of a write in
/// progress would leave its result unknown, so such a holder stays to the end of its save.
#[derive(Default)]
pub(super) struct ClusterKeys {
    shown: Option<KeysHolder>,
    leaving: Vec<KeysHolder>,
}

impl ClusterKeys {
    /// Holds the object of `target`, or no object. The same object again keeps its holder.
    pub(super) fn show(&mut self, target: Option<&Target>, start: impl FnOnce(&Target) -> KeysHolder) {
        if let (Some(holder), Some(target)) = (&self.shown, target)
            && &holder.target == target
        {
            return;
        }
        self.leaving.retain(KeysHolder::is_saving);
        // A holder with a save that waits or runs stays to the end of the save. Each other
        // holder stops here, and its values go.
        if let Some(old) = self.shown.take()
            && old.close()
        {
            self.leaving.push(old);
        }
        self.shown = target.map(start);
    }

    /// The holder of the shown object, when it is the object of `target`. The UID can
    /// differ: a view can ask for any object of the name.
    pub(super) fn holder(&self, target: &Target) -> Option<&KeysHolder> {
        let same = |holder: &&KeysHolder| {
            let held = &holder.target;
            (&held.cluster, held.kind, &held.key) == (&target.cluster, target.kind, &target.key)
        };
        self.shown.as_ref().filter(same)
    }

    /// The view of the shown object.
    pub(super) fn view(&self) -> Option<Arc<KeysView>> {
        self.shown.as_ref().map(|holder| Arc::clone(&lock(&holder.slot)))
    }
}

/// The holder of the object of a key view. Its task stops when this value drops.
pub(super) struct KeysHolder {
    target: Target,
    slot: Slot,
    saved: Arc<Notify>,
    closing: Arc<AtomicBool>,
    journal: Journal,
    _task: TaskHandle,
}

impl KeysHolder {
    /// Starts the task, which reads the object at once. `metadata` is the store of the
    /// metadata of its kind in its cluster.
    pub(super) fn start(
        inner: &Inner,
        target: Target,
        gateway: Gateway,
        metadata: Option<Reader<Snapshot<Meta>>>,
    ) -> Self {
        let first = KeysView { target: target.clone(), object: None, read: ReadState::Reading, save: None };
        let slot: Slot = Arc::new(Mutex::new(Arc::new(first)));
        let (saved, closing) = (Arc::new(Notify::new()), Arc::new(AtomicBool::new(false)));
        let name = TaskName::scoped("key view", target.cluster.as_str());
        let parts = Parts {
            gateway,
            metadata,
            slot: Arc::clone(&slot),
            saved: Arc::clone(&saved),
            closing: Arc::clone(&closing),
            repaint: inner.repaint.clone(),
            journal: inner.journal.clone(),
        };
        let mut runs = 0_u64;
        let task = inner.tasks.start(name, Restart::Always(Backoff::default()), move || {
            runs += 1;
            follow(parts.clone(), runs > 1)
        });
        Self { target, slot, saved, closing, journal: inner.journal.clone(), _task: task }
    }

    /// Gives a save to the task. The view shows it as pending at once. A save that waits
    /// for the task gives its place to this one: the view shows the last save. The same
    /// save again while it waits or runs, as from a second click, adds nothing: it would go
    /// with the old version after the first one, and come back as a conflict with it.
    pub(super) fn save(&self, number: u64, save: Save) {
        change(&self.slot, |view| {
            if view.save.as_ref().is_some_and(|job| job.state == WriteState::Pending && job.save == save) {
                return;
            }
            view.save = Some(SaveJob { number, save, state: WriteState::Pending });
        });
        self.saved.notify_one();
    }

    fn is_saving(&self) -> bool {
        pending(&self.slot).is_some()
    }

    /// Marks the holder of a view that goes to an other object, when a save waits or runs.
    /// Its task then ends the save, gives its end to the journal and drops the values. The
    /// check and the mark are under the lock of the view, as the end of a save is. True for
    /// such a holder.
    fn close(&self) -> bool {
        let view = lock(&self.slot);
        let saving = view.save.as_ref().is_some_and(|job| job.state == WriteState::Pending);
        self.closing.store(saving, Ordering::Release);
        saving
    }
}

impl Drop for KeysHolder {
    /// The task stops with this drop: the cluster left the selection, its connections went
    /// again, or the core stops. A save that waits or runs then has a result that nobody
    /// knows, and no view shows it any more, so the journal tells it (design rule 6).
    fn drop(&mut self) {
        if self.is_saving() {
            let text = format!(
                "{} stopped with the connections of its cluster: its result is not known",
                save_of(&self.target)
            );
            self.journal.warning(self.target.cluster.as_str(), text);
        }
    }
}
