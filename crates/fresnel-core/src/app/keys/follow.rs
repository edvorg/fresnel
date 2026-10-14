//! The task of one holder of a key view: a read, then a wait for a reason to read again.
//! A save comes during the wait, and its answer is the new object.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use fresnel_net::ErrorClass;
use tokio::sync::Notify;
use tokio::time::{Instant, sleep_until};

use super::{KeysView, ReadState};
use crate::app::repaint::Repaint;
use crate::cluster::Gateway;
use crate::error::CoreError;
use crate::keys::{self, Save, Target, WriteState};
use crate::kinds::{Meta, Object};
use crate::published::Reader;
use crate::store::Snapshot;

/// The first pause before a new read after a read that failed for a reason that can pass.
const FIRST_PAUSE: Duration = Duration::from_secs(1);
/// The longest such pause.
const MAX_PAUSE: Duration = Duration::from_secs(30);

/// The view, which the task writes and the core copies.
pub(super) type Slot = Arc<Mutex<Arc<KeysView>>>;

pub(super) fn lock(slot: &Slot) -> MutexGuard<'_, Arc<KeysView>> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Changes a copy of the view, and puts the copy into the slot. A reader keeps its copy.
pub(super) fn change(slot: &Slot, edit: impl FnOnce(&mut KeysView)) {
    let mut guard = lock(slot);
    let mut view = KeysView::clone(&guard);
    edit(&mut view);
    *guard = Arc::new(view);
}

/// The save that waits for the task, with its number.
pub(super) fn pending(slot: &Slot) -> Option<(u64, Save)> {
    let job = lock(slot).save.clone()?;
    (job.state == WriteState::Pending).then_some((job.number, job.save))
}

/// What one run of the task needs.
#[derive(Clone)]
pub(super) struct Parts {
    pub(super) gateway: Gateway,
    /// The metadata store of the kind of the object in its cluster.
    pub(super) metadata: Option<Reader<Snapshot<Meta>>>,
    pub(super) slot: Slot,
    /// Wakes the task for a save. The pending job of the slot is the save to send.
    pub(super) saved: Arc<Notify>,
    /// Set when the view shows an other object during a save: the task ends its work after
    /// the save, and drops the values.
    pub(super) closing: Arc<AtomicBool>,
    pub(super) repaint: Repaint,
}

/// The body of the task.
pub(super) async fn follow(parts: Parts, restarted: bool) -> Result<(), CoreError> {
    let Parts { gateway, mut metadata, slot, saved, closing, repaint } = parts;
    let target = lock(&slot).target.clone();
    // The run before this one can have ended during a save, which can be in effect. The
    // first read shows the result, and the save does not go again (design rule 6).
    if restarted {
        change(&slot, |view| {
            if let Some(job) = view.save.as_mut().filter(|job| job.state == WriteState::Pending) {
                job.state = WriteState::Unknown(CoreError::invalid("the key view started again during the save"));
            }
        });
    }
    let mut seen = in_store(metadata.as_ref(), &target);
    let mut pause = FIRST_PAUSE;
    loop {
        change(&slot, |view| view.read = ReadState::Reading);
        repaint.request();
        let read = keys::read(&gateway, &target).await;
        let retry_at = match &read {
            Err(error) if can_pass(error) => Some(Instant::now() + pause),
            Ok(_) | Err(_) => None,
        };
        pause = if read.is_ok() { FIRST_PAUSE } else { (pause * 2).min(MAX_PAUSE) };
        change(&slot, |view| match read {
            Ok(object) => (view.object, view.read) = (Some(Arc::new(object)), ReadState::Read),
            Err(error) => view.read = ReadState::Failed(error),
        });
        repaint.request();
        loop {
            if let Some((number, save)) = pending(&slot) {
                let read_again = run_save(&gateway, number, &save, &slot, &repaint).await;
                if closing.load(Ordering::Acquire) {
                    change(&slot, |view| (view.object, view.save) = (None, None));
                    return std::future::pending().await;
                }
                if read_again {
                    break;
                }
                continue;
            }
            let now = in_store(metadata.as_ref(), &target);
            if now != seen {
                seen = now;
                let loaded = lock(&slot).object.as_ref().map(|object| Arc::clone(&object.version));
                if seen != loaded {
                    break;
                }
            }
            let store_changed = async {
                match metadata.as_mut() {
                    Some(reader) => reader.changed().await,
                    None => std::future::pending().await,
                }
            };
            let retry = async {
                match retry_at {
                    Some(at) => sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                () = saved.notified() => {}
                () = store_changed => {}
                () = retry => break,
            }
        }
    }
}

/// Sends one save and shows its result. True when the object needs a new read: after a
/// conflict, and after a save whose result is not known (design rule 6).
async fn run_save(gateway: &Gateway, number: u64, save: &Save, slot: &Slot, repaint: &Repaint) -> bool {
    let state = WriteState::of_save(keys::save(gateway, save).await);
    let read_again = matches!(state, WriteState::Conflict(_) | WriteState::Unknown(_));
    change(slot, |view| {
        if let WriteState::Done(object) = &state {
            (view.object, view.read) = (Some(Arc::clone(object)), ReadState::Read);
        }
        if let Some(job) = view.save.as_mut().filter(|job| job.number == number) {
            job.state = state;
        }
    });
    repaint.request();
    read_again
}

/// The resource version of the object in the metadata store. `None` for an object that the
/// store does not have.
fn in_store(metadata: Option<&Reader<Snapshot<Meta>>>, target: &Target) -> Option<Arc<str>> {
    let snapshot = metadata?.read();
    snapshot.items.get(&target.key)?.meta().resource_version.as_deref().map(Arc::from)
}

/// True for a failed read that a later read can pass: the network, a time limit or the
/// server. A refusal or a missing object waits for a change in the store.
fn can_pass(error: &CoreError) -> bool {
    matches!(error.class(), ErrorClass::Network | ErrorClass::Timeout | ErrorClass::Server)
}
