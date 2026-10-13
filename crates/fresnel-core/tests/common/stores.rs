//! The stores of one cluster in a test, and the wait for their states.

use std::time::Duration;

use fresnel_core::cluster::Link;
use fresnel_core::index::{ClusterIndexes, IndexParts};
use fresnel_core::net::WallClock;
use fresnel_core::published::Disposal;
use fresnel_core::store::{ClusterStores, StoreParts, StoreSettings, StoreStatus};
use tokio::time::timeout;

use crate::common::TestTasks;

/// The stores of one cluster of a test, with the thread that frees their old snapshots.
/// The stores are the first field: they stop before the thread.
pub(crate) struct Stores {
    pub(crate) stores: ClusterStores,
    disposal: Disposal,
}

impl Stores {
    pub(crate) fn start(tasks: &TestTasks, link: &Link, settings: StoreSettings) -> Self {
        let disposal = Disposal::start(&tasks.tasks);
        let parts = StoreParts { settings, bin: disposal.bin(), clock: WallClock::system() };
        let stores = ClusterStores::start(&tasks.tasks, link, parts).expect("good settings");
        Self { stores, disposal }
    }

    /// Starts the indexes on these stores. They must drop before the stores.
    pub(crate) fn indexes(&self, tasks: &TestTasks) -> ClusterIndexes {
        let parts = IndexParts { bin: self.disposal.bin(), restart: StoreSettings::default().restart };
        ClusterIndexes::start(&tasks.tasks, &self.stores, parts).expect("each store of an index")
    }

    /// Waits until the facts of the stores meet the condition. `limit` is the longest wait.
    pub(crate) async fn until(
        &self,
        what: &str,
        limit: Duration,
        met: impl Fn(&[StoreStatus]) -> bool,
    ) -> Vec<StoreStatus> {
        let mut changes = self.stores.changes();
        let wait = async {
            loop {
                let statuses = self.stores.statuses();
                if met(&statuses) {
                    return statuses;
                }
                changes.next().await;
            }
        };
        timeout(limit, wait).await.unwrap_or_else(|_| panic!("in time: {what}: {:#?}", self.states()))
    }

    /// Waits until each store is live.
    pub(crate) async fn until_live(&self, limit: Duration) -> Vec<StoreStatus> {
        self.until("each store is live", limit, |statuses| statuses.iter().all(|status| status.state.is_live())).await
    }

    /// The kind and the state of each store, in words.
    pub(crate) fn states(&self) -> Vec<String> {
        let line =
            |status: &StoreStatus| format!("{}: {} ({} objects)", status.kind.info().name, status.state, status.items);
        self.stores.statuses().iter().map(line).collect()
    }
}
