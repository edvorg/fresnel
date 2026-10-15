//! The copy of the core that the tasks of the soak read, as a view of the window reads it.
//!
//! One task takes a copy at each repaint that the core asks for, and each second when no
//! repaint comes, and shares it. A check then waits for a new copy, not for a fixed time.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use fresnel_core::app::{ClusterRow, Core};
use fresnel_core::cluster::ClusterId;
use fresnel_core::journal::JournalView;
use fresnel_core::kinds::Kind;
use fresnel_core::net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use fresnel_core::store::{StoreState, StoreStatus};
use tokio::sync::{Notify, watch};
use tokio::time::{Instant, sleep};

use super::app::App;
use super::cover::{Version, Versions};

/// The longest time between two copies.
const LONGEST: Duration = Duration::from_secs(1);

/// The core at one moment.
pub(super) struct Look {
    /// When the copy was taken.
    pub(super) at: Instant,
    /// The cluster of the soak. `None` before the core read the kubeconfig files.
    pub(super) row: Option<ClusterRow>,
    pub(super) journal: Arc<JournalView>,
}

/// The store of one kind at one moment.
pub(super) struct Store {
    pub(super) state: StoreState,
    pub(super) dropped: u64,
    pub(super) versions: Versions,
}

impl Look {
    /// The facts of the store of each kind that is always on, in the sequence of the kinds.
    pub(super) fn statuses(&self) -> &[StoreStatus] {
        self.row.as_ref().and_then(|row| row.data.as_ref()).map_or(&[], |data| &data.stores)
    }

    /// The objects of the store of a kind that is always on. `None` while the cluster has no
    /// stores.
    pub(super) fn store(&self, kind: Kind) -> Option<Store> {
        let data = self.row.as_ref()?.data.as_ref()?;
        let source = data.sources.iter().find(|source| source.status.kind == kind)?;
        let mut versions = Versions::new();
        source.each_meta(&mut |key, meta| {
            let version = Version::new(meta.uid.as_deref(), meta.resource_version.as_deref());
            versions.insert((Arc::clone(&key.namespace), Arc::clone(&key.name)), version);
        });
        Some(Store { state: source.status.state.clone(), dropped: source.status.dropped, versions })
    }
}

/// Starts the task of the copies. It stops when the handle drops.
pub(super) fn start(tasks: &Tasks, app: &App) -> (TaskHandle, watch::Receiver<Arc<Look>>) {
    let (core, id, changed) = (Arc::clone(&app.core), app.id.clone(), Arc::clone(&app.changed));
    let (sender, receiver) = watch::channel(Arc::new(take(&core, &id)));
    let name = TaskName::new("soak copy of the core");
    let task = tasks.start(name, Restart::Always(Backoff::default()), move || {
        copies(Arc::clone(&core), id.clone(), Arc::clone(&changed), sender.clone())
    });
    (task, receiver)
}

async fn copies(
    core: Arc<Core>,
    id: ClusterId,
    changed: Arc<Notify>,
    sender: watch::Sender<Arc<Look>>,
) -> Result<(), Infallible> {
    loop {
        // A quiet cluster asks for no repaint: its copy comes after the longest time.
        tokio::select! {
            () = changed.notified() => {}
            () = sleep(LONGEST) => {}
        }
        sender.send_replace(Arc::new(take(&core, &id)));
    }
}

fn take(core: &Core, id: &ClusterId) -> Look {
    let view = core.view();
    let row = view.clusters.into_iter().find(|row| &row.id == id);
    Look { at: Instant::now(), row, journal: view.journal }
}
