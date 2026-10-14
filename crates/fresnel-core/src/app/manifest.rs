//! The read of the manifest that the detail panel shows (plan section 12.9).
//!
//! One task reads the manifest that the view asked for last: the panel shows one object,
//! so a new request ends the read in progress. The task does not end after a read, so a
//! read gives no journal entry. A read that fails shows in the panel with its reason.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use tokio::sync::watch;

use super::repaint::Repaint;
use crate::cluster::{ClusterId, Registry};
use crate::error::CoreError;
use crate::kinds::Kind;
use crate::manifest::{self, ManagedFields, Manifest};
use crate::store::ObjectKey;

/// The manifest that a view asks for: one object of one cluster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestRequest {
    /// The cluster of the object.
    pub cluster: ClusterId,
    /// The kind of the object.
    pub kind: Kind,
    /// The namespace and the name of the object.
    pub key: ObjectKey,
    /// The UID of the object, or empty for any object of the name. A read that finds an
    /// other object of the name fails.
    pub uid: Arc<str>,
    /// What the manifest shows of `managedFields`.
    pub managed_fields: ManagedFields,
}

/// The last read of a manifest that a view asked for.
#[derive(Clone, Debug)]
pub struct ManifestRead {
    /// What the view asked for.
    pub request: ManifestRequest,
    /// A number that grows with each request, also with a request for the same object.
    pub number: u64,
    /// How far the read is.
    pub state: ManifestState,
}

/// How far the read of a manifest is.
#[derive(Clone, Debug)]
pub enum ManifestState {
    /// The request is on its way, with the time limit of a GET.
    Reading,
    /// The manifest as the server gave it.
    Done(Arc<Manifest>),
    /// Why the read failed.
    Failed(CoreError),
}

/// The task that reads the manifests, with the last read. The task stops when this value
/// drops.
pub(super) struct ManifestReader {
    _task: TaskHandle,
    wanted: watch::Sender<Option<(u64, ManifestRequest)>>,
    last: Last,
    number: u64,
}

/// The last read, which the task writes and the view copies.
type Last = Arc<Mutex<Option<Arc<ManifestRead>>>>;

fn lock(last: &Last) -> MutexGuard<'_, Option<Arc<ManifestRead>>> {
    last.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ManifestReader {
    /// Starts the task. It reads through the gateway of the cluster of each request.
    pub(super) fn start(tasks: &Tasks, registry: Registry, repaint: Repaint) -> Self {
        let wanted = watch::Sender::new(None);
        let last = Last::default();
        let task = tasks.start(TaskName::new("manifest reader"), Restart::Always(Backoff::default()), {
            let (wanted, last) = (wanted.clone(), Arc::clone(&last));
            move || serve(wanted.subscribe(), Arc::clone(&last), registry.clone(), repaint.clone())
        });
        Self { _task: task, wanted, last, number: 0 }
    }

    /// Asks for a manifest. A read in progress for an older request ends.
    pub(super) fn ask(&mut self, request: ManifestRequest) {
        self.number += 1;
        let read = ManifestRead { request: request.clone(), number: self.number, state: ManifestState::Reading };
        *lock(&self.last) = Some(Arc::new(read));
        self.wanted.send_replace(Some((self.number, request)));
    }

    /// The last read, for the view.
    pub(super) fn last(&self) -> Option<Arc<ManifestRead>> {
        lock(&self.last).clone()
    }
}

/// The body of the task: reads the newest request, and starts again when a newer one
/// comes during the read.
async fn serve(
    mut wanted: watch::Receiver<Option<(u64, ManifestRequest)>>,
    last: Last,
    registry: Registry,
    repaint: Repaint,
) -> Result<(), CoreError> {
    let closed = || CoreError::invalid("the core stopped");
    loop {
        let Some((number, request)) = wanted.borrow_and_update().clone() else {
            wanted.changed().await.map_err(|_| closed())?;
            continue;
        };
        let state = match registry.link(&request.cluster) {
            None => ManifestState::Failed(CoreError::invalid(format!("{} is not connected", request.cluster))),
            Some(link) => {
                let read =
                    manifest::fetch(&link.gateway, request.kind, &request.key, &request.uid, request.managed_fields);
                tokio::select! {
                    result = read => result.map_or_else(ManifestState::Failed, |manifest| {
                        ManifestState::Done(Arc::new(manifest))
                    }),
                    changed = wanted.changed() => {
                        changed.map_err(|_| closed())?;
                        continue;
                    }
                }
            }
        };
        {
            let mut slot = lock(&last);
            // A newer request took the place of this one during the last lines.
            if slot.as_ref().is_some_and(|read| read.number == number) {
                *slot = Some(Arc::new(ManifestRead { request, number, state }));
            }
        }
        repaint.request();
        wanted.changed().await.map_err(|_| closed())?;
    }
}
