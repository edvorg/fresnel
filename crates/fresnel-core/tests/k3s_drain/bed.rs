//! Two k3s clusters and the app core with the contexts `a` and `b`. Cluster `b` has the node
//! of its server and two agents, `drained` and `twin`, with a taint that keeps the pods of
//! k3s on the server: the app drains `drained`, and kubectl drains `twin` with the same
//! options. The node of the server of cluster `a` has the name `drained` too.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use fresnel_core::app::{Command, Core, CoreParts, CoreSettings, CoreView, JobView};
use fresnel_core::cluster::{ClusterId, Link, Registry};
use fresnel_core::journal::Journal;
use fresnel_core::kinds::{Kind, Object};
use fresnel_core::net::{NetSettings, WallClock};
use fresnel_core::ops::{Operation, Order, Target};
use fresnel_core::store::ObjectKey;
use fresnel_testkit::{Home, K3s, K3sOptions};
use k8s_openapi::api::core::v1::Node;
use serde_json::Value as Json;
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::common::{TestTasks, connect, registry, until_live};
use crate::contexts::kubeconfig;

/// The longest wait for a state of the core or of a cluster.
pub(crate) const LIMIT: Duration = Duration::from_secs(240);
/// The node that the app drains, and its twin that kubectl drains.
pub(crate) const DRAINED: &str = "drained";
pub(crate) const TWIN: &str = "twin";
/// The taint of the two agents: only the pods of the scene tolerate it.
pub(crate) const TAINT: &str = "fresnel/drain";

/// One cluster: its container and a connection of its admin, for the scene.
pub(crate) struct Cluster {
    pub(crate) admin: Link,
    pub(crate) k3s: K3s,
}

/// The core and the clusters. The fields drop in this sequence: the core stops before the
/// connections of the admin, and both before the containers.
pub(crate) struct Bed {
    pub(crate) core: Core,
    woken: Arc<Notify>,
    _home: Home,
    _setup: (Registry, Registry, TestTasks),
    pub(crate) a: Cluster,
    pub(crate) b: Cluster,
}

impl Bed {
    pub(crate) async fn start() -> Self {
        let taint = format!("--node-taint={TAINT}=yes:NoSchedule");
        let agents = K3sOptions::default().agent(DRAINED, &[&taint]).agent(TWIN, &[&taint]);
        let a = K3s::start_with("m7-drain-a", K3sOptions::default().node_name(DRAINED));
        let (a, b) = tokio::join!(a, K3s::start_with("m7-drain-b", agents));
        let (a, b) = (a.expect("k3s a starts"), b.expect("k3s b starts"));
        let tasks = TestTasks::new(&Handle::current());
        let setup = |k3s: &K3s| registry(&tasks, &Handle::current(), &k3s.kubeconfig(), NetSettings::default());
        let (registry_a, registry_b) = (setup(&a), setup(&b));
        let (admin_a, admin_b) = (connect(&registry_a, "default"), connect(&registry_b, "default"));
        until_live(&admin_a).await;
        until_live(&admin_b).await;

        let home = Home::new("m7-drain", &["kubeconfig.yaml"]).expect("a home folder");
        let contexts = [("a", a.kubeconfig()), ("b", b.kubeconfig())];
        home.write("work/kubeconfig.yaml", &kubeconfig(&contexts)).expect("the kubeconfig");
        let core = Core::new(CoreParts {
            runtime: Handle::current(),
            base_env: home.finder_env(),
            journal: Journal::new(10_000, WallClock::system()),
            state_file: home.path("state/state.json"),
            config_file: home.path("state/config.toml"),
            downloads: home.path("Downloads"),
            detectors: Vec::new(),
            settings: CoreSettings::default(),
        });
        let woken = Arc::new(Notify::new());
        core.start({
            let woken = Arc::clone(&woken);
            move || woken.notify_one()
        });
        let a = Cluster { admin: admin_a, k3s: a };
        let b = Cluster { admin: admin_b, k3s: b };
        let bed = Self { core, woken, _home: home, _setup: (registry_a, registry_b, tasks), a, b };
        // The first start selects the current context `a`. The page of the Nodes shows them
        // and the DaemonSets that its drain dialog reads.
        bed.core.send(Command::Select { cluster: ClusterId::new("b"), selected: true });
        bed.core.send(Command::ShowKinds([Kind::Node, Kind::DaemonSet].into_iter().collect()));
        bed
    }

    /// Waits until the view meets the condition, at each wake of the UI, as the window does.
    pub(crate) async fn until(&self, what: &str, met: impl Fn(&CoreView) -> bool) -> CoreView {
        let wait = async {
            loop {
                let view = self.core.view();
                if met(&view) {
                    return view;
                }
                self.woken.notified().await;
            }
        };
        timeout(LIMIT, wait).await.unwrap_or_else(|_| {
            let view = self.core.view();
            let jobs: Vec<&JobView> = view.jobs.jobs.iter().map(|job| &**job).collect();
            panic!("in time: {what}\njobs: {jobs:#?}")
        })
    }

    /// The target of a node of a cluster with its UID in the store of the core, as the page
    /// of the Nodes takes it from the row that the user selected.
    pub(crate) async fn node(&self, cluster: &str, name: &str) -> Target {
        let key = ObjectKey::new("", name);
        let has = |view: &CoreView| node_of(view, cluster, &key).is_some();
        let view = self.until(&format!("the node {name} of {cluster} in the store"), has).await;
        let uid = node_of(&view, cluster, &key).and_then(|node| node.meta().uid.clone()).expect("a UID");
        Target { cluster: ClusterId::new(cluster), kind: Kind::Node, key, uid: uid.into() }
    }

    /// Sends one order as `Command::Run`, as the dialog does. Gives the number of its job.
    pub(crate) fn send(&self, order: Order) -> u64 {
        let number = self.core.view().jobs.jobs.last().map_or(1, |job| job.number + 1);
        self.core.send(Command::Run(vec![order]));
        number
    }

    /// Runs an operation on a node to the final state of its job.
    pub(crate) async fn run(&self, cluster: &str, node: &str, operation: Operation) -> Arc<JobView> {
        let order = Order::new(self.node(cluster, node).await, operation).expect("an order");
        let number = self.send(order);
        self.ended(number).await
    }

    /// Waits for the final state of the job with this number.
    pub(crate) async fn ended(&self, number: u64) -> Arc<JobView> {
        let done = |view: &CoreView| job(view, number).is_some_and(|job| job.report.state.is_final());
        let view = self.until(&format!("the end of job {number}"), done).await;
        job(&view, number).cloned().expect("the job")
    }
}

/// The job with this number.
pub(crate) fn job(view: &CoreView, number: u64) -> Option<&Arc<JobView>> {
    view.jobs.jobs.iter().find(|job| job.number == number)
}

fn node_of(view: &CoreView, cluster: &str, key: &ObjectKey) -> Option<Arc<Node>> {
    let row = view.clusters.iter().find(|row| row.id.as_str() == cluster)?;
    let nodes = row.data.as_ref()?.snapshot::<Node>()?;
    nodes.items.get(key).cloned()
}

impl Cluster {
    /// Runs kubectl as the cluster admin. A refusal is a panic.
    pub(crate) async fn kubectl(&self, arguments: &[&str]) -> String {
        self.k3s.kubectl(arguments).await.unwrap_or_else(|error| panic!("kubectl {arguments:?}: {error}"))
    }

    /// The label `app` of each pod on a node by the name of the pod: the pods of the scene
    /// and the pods of k3s, with no pod that ends now.
    pub(crate) async fn pods_on(&self, node: &str) -> BTreeMap<String, String> {
        let selector = format!("spec.nodeName={node}");
        let list =
            self.kubectl(&["get", "pods", "--all-namespaces", "--field-selector", &selector, "-o", "json"]).await;
        let list: Json = serde_json::from_str(&list).expect("JSON");
        let pods = list["items"].as_array().cloned().unwrap_or_default();
        let running = pods.iter().filter(|pod| pod.pointer("/metadata/deletionTimestamp").is_none());
        let field = |pod: &Json, path: &str| pod.pointer(path).and_then(Json::as_str).unwrap_or_default().to_owned();
        running.map(|pod| (field(pod, "/metadata/name"), field(pod, "/metadata/labels/app"))).collect()
    }

    /// True when the node takes no new pod.
    pub(crate) async fn cordoned(&self, node: &str) -> bool {
        self.kubectl(&["get", "node", node, "-o", "jsonpath={.spec.unschedulable}"]).await.trim() == "true"
    }
}
