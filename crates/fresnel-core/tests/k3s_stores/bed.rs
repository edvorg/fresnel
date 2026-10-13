//! The cluster of one test, and the app before it.

use std::time::{Duration, SystemTime};

use fresnel_core::cluster::{Link, Registry};
use fresnel_core::net::NetSettings;
use fresnel_core::store::{StoreSettings, StoreStatus};
use fresnel_testkit::{FaultProxy, K3s};
use tokio::runtime::Handle;

use crate::common::{TestTasks, connect, registry, until_live};
use crate::store_bed::Stores;

/// The longest wait for a state with no time of its own in the acceptance.
pub(crate) const LIMIT: Duration = Duration::from_secs(60);
/// Plan section 15: each store is live 10 seconds or less after the network returns.
pub(crate) const RECOVERY: Duration = Duration::from_secs(10);
/// Plan section 6.8: a connection that carries nothing shows in 20 seconds or less.
pub(crate) const SHOWS_IN: Duration = Duration::from_secs(20);

/// A k3s cluster with a connection of the test itself. The fields drop in this sequence:
/// the connection goes before the cluster.
pub(crate) struct Cluster {
    /// The API with no proxy: the reads and the writes of the test.
    pub(crate) api: Link,
    _registry: Registry,
    pub(crate) tasks: TestTasks,
    pub(crate) k3s: K3s,
}

impl Cluster {
    pub(crate) async fn start(label: &str) -> Self {
        let k3s = K3s::start(label).await.expect("k3s starts");
        let tasks = TestTasks::new(&Handle::current());
        let registry = registry(&tasks, &Handle::current(), &k3s.kubeconfig(), NetSettings::default());
        let api = connect(&registry, "default");
        until_live(&api).await;
        Self { api, _registry: registry, tasks, k3s }
    }
}

/// The app for one cluster: its stores, on connections through a fault proxy of its own.
/// The stores drop first, and the proxy last.
pub(crate) struct App {
    pub(crate) stores: Stores,
    pub(crate) link: Link,
    _registry: Registry,
    pub(crate) proxy: FaultProxy,
}

impl App {
    /// Connects through a new proxy and starts each store.
    pub(crate) async fn start(cluster: &Cluster, settings: StoreSettings) -> Self {
        let proxy = FaultProxy::start(cluster.k3s.api_addr()).await.expect("a proxy");
        let kubeconfig = cluster.k3s.kubeconfig_through(proxy.addr()).expect("a kubeconfig");
        let registry = registry(&cluster.tasks, &Handle::current(), &kubeconfig, NetSettings::default());
        let link = connect(&registry, "default");
        let stores = Stores::start(&cluster.tasks, &link, settings);
        Self { stores, link, _registry: registry, proxy }
    }

    /// Waits until each store is live again after a fault that cut each watch: each store
    /// then has a state from after the fault. Gives the time from `fault` to that moment.
    pub(crate) async fn live_again_after(&self, fault: SystemTime, limit: Duration) -> Duration {
        let again = |stores: &[StoreStatus]| stores.iter().all(|store| store.state.is_live() && store.since >= fault);
        let stores = self.stores.until("each store is live again", limit, again).await;
        let last = stores.iter().map(|store| store.since).max().unwrap_or(fault);
        last.duration_since(fault).unwrap_or_default()
    }

    /// Stops each store and starts new ones on the same connections: each one lists again.
    pub(crate) fn start_stores_again(&mut self, cluster: &Cluster, settings: StoreSettings) {
        self.stores = Stores::start(&cluster.tasks, &self.link, settings);
    }
}

/// True when no store shows its objects as current.
pub(crate) fn none_live(stores: &[StoreStatus]) -> bool {
    stores.iter().all(|store| !store.state.is_live())
}
