//! Three k3s clusters in one registry, as the contexts `a`, `b` and `c`, with the stores, the
//! indexes and a sampler of each one. `a` and `b` run metrics-server and have a node of the
//! same name; `c` has no metrics-server.

use std::time::Duration;

use fresnel_core::cluster::{ClusterId, Link, Registry};
use fresnel_core::index::ClusterIndexes;
use fresnel_core::kinds::Kind;
use fresnel_core::kubeconfig;
use fresnel_core::metrics::{Inputs, Metrics, MetricsSettings, Sampler, SamplerParts};
use fresnel_core::net::env::ProcessEnv;
use fresnel_core::net::{NetSettings, WallClock};
use fresnel_core::published::{Disposal, Notice, Reader};
use fresnel_core::store::StoreSettings;
use fresnel_testkit::{K3s, K3sOptions};
use kube::core::Request;
use kube::core::params::PostParams;
use serde_json::Value as Json;
use tokio::runtime::Handle;
use tokio::time::timeout;

use crate::common::{TestTasks, connect, until_live};
use crate::store_bed::Stores;

/// The name of the node of `a` and of `b`.
pub(crate) const NODE: &str = "m6-node";
/// The longest wait for a state of the metrics. metrics-server gives its first values one
/// or two scrapes of 15 s after its start, and its image can need a pull.
pub(crate) const LIMIT: Duration = Duration::from_secs(300);

/// One cluster. The sampler drops before the indexes and the stores that it reads, and
/// those before the connection and the container.
pub(crate) struct Cluster {
    pub(crate) id: ClusterId,
    pub(crate) metrics: Reader<Metrics>,
    _sampler: Sampler,
    indexes: ClusterIndexes,
    stores: Stores,
    link: Link,
    k3s: K3s,
}

/// The three clusters of the test.
pub(crate) struct Bed {
    pub(crate) a: Cluster,
    pub(crate) b: Cluster,
    pub(crate) c: Cluster,
    _registry: Registry,
    disposal: Disposal,
    tasks: TestTasks,
}

impl Bed {
    pub(crate) async fn start() -> Self {
        let measured = || K3sOptions::default().metrics_server().node_name(NODE);
        let (a, b, c) = tokio::join!(
            K3s::start_with("m6-metrics-a", measured()),
            K3s::start_with("m6-metrics-b", measured()),
            K3s::start("m6-metrics-c"),
        );
        let (a, b, c) = (a.expect("k3s a starts"), b.expect("k3s b starts"), c.expect("k3s c starts"));
        let tasks = TestTasks::new(&Handle::current());
        let env: ProcessEnv = [("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")].into_iter().collect();
        let registry = Registry::new(tasks.tasks.clone(), Handle::current(), env, NetSettings::default());
        // Each kubeconfig file names its context `default`. The names `a`, `b` and `c` are the
        // identities of the clusters in the app, as contexts of the user.
        let contexts = [("a", &a), ("b", &b), ("c", &c)].map(|(name, k3s)| {
            let mut loaded = kubeconfig::load(&[k3s.kubeconfig()]);
            assert_eq!(loaded.problems, [], "the kubeconfig of {name}");
            let mut context = loaded.contexts.remove(0);
            context.name = name.to_owned();
            context
        });
        registry.set_contexts(contexts.into());
        let disposal = Disposal::start(&tasks.tasks);
        let cluster = |name: &str, k3s: K3s| {
            let link = connect(&registry, name);
            let stores = Stores::start(&tasks, &link, StoreSettings::default());
            let indexes = stores.indexes(&tasks);
            let sampler = start(&tasks, &disposal, (&link, &stores, &indexes), MetricsSettings::default());
            let metrics = sampler.reader();
            Cluster { id: ClusterId::new(name), metrics, _sampler: sampler, indexes, stores, link, k3s }
        };
        let (a, b, c) = (cluster("a", a), cluster("b", b), cluster("c", c));
        for cluster in [&a, &b, &c] {
            until_live(&cluster.link).await;
            cluster.stores.until_live(LIMIT).await;
        }
        Self { a, b, c, _registry: registry, disposal, tasks }
    }

    /// One more sampler of a cluster, with its own settings and its own history.
    pub(crate) fn sampler(&self, cluster: &Cluster, settings: MetricsSettings) -> Sampler {
        start(&self.tasks, &self.disposal, (&cluster.link, &cluster.stores, &cluster.indexes), settings)
    }
}

fn start(
    tasks: &TestTasks,
    disposal: &Disposal,
    (link, stores, indexes): (&Link, &Stores, &ClusterIndexes),
    settings: MetricsSettings,
) -> Sampler {
    let inputs = Inputs::of(&stores.stores, indexes).expect("a store of each kind");
    let parts = SamplerParts { settings, bin: disposal.bin(), notice: Notice::new(), clock: WallClock::system() };
    Sampler::start(&tasks.tasks, link.gateway.clone(), inputs, parts)
}

impl Cluster {
    /// Creates an object of a kind in the namespace `default`.
    pub(crate) async fn create(&self, kind: Kind, object: &Json) {
        let body = serde_json::to_vec(object).expect("JSON");
        let request = Request::new(kind.info().path_in("default")).create(&PostParams::default(), body);
        let answer: Result<Json, _> = self.link.gateway.send(request.expect("a request")).await;
        answer.unwrap_or_else(|error| panic!("the create in {}: {error}", self.id));
    }

    /// Runs kubectl in the cluster.
    pub(crate) async fn kubectl(&self, arguments: &[&str]) -> String {
        self.k3s.kubectl(arguments).await.unwrap_or_else(|error| panic!("kubectl {arguments:?}: {error}"))
    }
}

/// Waits until the metrics of a reader meet the condition, and gives them.
pub(crate) async fn until(reader: &Reader<Metrics>, what: &str, met: impl Fn(&Metrics) -> bool) -> Metrics {
    let mut reader = reader.clone();
    let wait = async {
        loop {
            let metrics = Metrics::clone(&reader.read());
            if met(&metrics) {
                return metrics;
            }
            reader.changed().await;
        }
    };
    let found = timeout(LIMIT, wait).await;
    found.unwrap_or_else(|_| {
        let metrics = reader.read();
        panic!("in time: {what} in {}: pods {}, nodes {}", metrics.cluster, metrics.pods.state, metrics.nodes.state)
    })
}
