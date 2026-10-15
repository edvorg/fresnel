//! Two k3s clusters in one registry, as the contexts `a` and `b`, with the stores of each one,
//! and the comparison of an object with its twin that kubectl changed.

use std::time::Duration;

use fresnel_core::cluster::{ClusterId, Link, Registry};
use fresnel_core::kinds::Kind;
use fresnel_core::kubeconfig;
use fresnel_core::net::NetSettings;
use fresnel_core::net::env::ProcessEnv;
use fresnel_core::ops::{self, Cancel, JobState, Operation, Order, Parts, Progress, Report, Target};
use fresnel_core::store::{ObjectKey, StoreSettings};
use fresnel_testkit::K3s;
use serde_json::Value as Json;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, sleep};

use crate::common::{TestTasks, connect, until_live};
use crate::store_bed::Stores;

/// The namespace of the objects that the app changes.
pub(crate) const APP: &str = "app";
/// The namespace of their twins, which kubectl changes in the same way.
pub(crate) const KUBECTL: &str = "kubectl";
/// The longest wait for a state of the cluster.
pub(crate) const LIMIT: Duration = Duration::from_secs(180);
/// The pause between two reads of a state that has no event for the test.
const STEP: Duration = Duration::from_millis(250);

/// One cluster: the container, the connection of the app and its stores. The stores drop
/// before the connection, and the connection before the container.
pub(crate) struct Cluster {
    pub(crate) id: ClusterId,
    pub(crate) stores: Stores,
    pub(crate) link: Link,
    pub(crate) k3s: K3s,
}

/// The two clusters of the test.
pub(crate) struct Bed {
    pub(crate) a: Cluster,
    pub(crate) b: Cluster,
    _registry: Registry,
    pub(crate) tasks: TestTasks,
}

impl Bed {
    pub(crate) async fn start() -> Self {
        let (a, b) = tokio::join!(K3s::start("m3-ops-a"), K3s::start("m3-ops-b"));
        let (a, b) = (a.expect("k3s a starts"), b.expect("k3s b starts"));
        let tasks = TestTasks::new(&Handle::current());
        let env: ProcessEnv = [("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")].into_iter().collect();
        let registry = Registry::new(tasks.tasks.clone(), Handle::current(), env, NetSettings::default());
        // Both kubeconfig files name their context `default`. The names `a` and `b` are the
        // identities of the two clusters in the app, as two contexts of the user.
        let contexts = [("a", &a), ("b", &b)].map(|(name, k3s)| {
            let mut loaded = kubeconfig::load(&[k3s.kubeconfig()]);
            assert_eq!(loaded.problems, [], "the kubeconfig of {name}");
            let mut context = loaded.contexts.remove(0);
            context.name = name.to_owned();
            context
        });
        registry.set_contexts(contexts.into());
        let cluster = |name: &str, k3s: K3s| {
            let link = connect(&registry, name);
            let stores = Stores::start(&tasks, &link, StoreSettings::default());
            Cluster { id: ClusterId::new(name), stores, link, k3s }
        };
        let (a, b) = (cluster("a", a), cluster("b", b));
        for cluster in [&a, &b] {
            until_live(&cluster.link).await;
            cluster.stores.until_live(LIMIT).await;
        }
        Self { a, b, _registry: registry, tasks }
    }
}

impl Cluster {
    /// The target of an object of the namespace of the app, with its UID of now.
    pub(crate) async fn target(&self, kind: Kind, name: &str) -> Target {
        let object = self.object(APP, kind, name).await.unwrap_or_else(|| panic!("{name} exists"));
        let uid = object.pointer("/metadata/uid").and_then(Json::as_str).expect("a UID");
        Target { cluster: self.id.clone(), kind, key: ObjectKey::new(APP, name), uid: uid.into() }
    }

    /// Runs an operation on an object of the namespace of the app to its end, with the
    /// parts of this cluster.
    pub(crate) async fn run(&self, kind: Kind, name: &str, operation: Operation) -> Report {
        let order = Order::new(self.target(kind, name).await, operation).expect("an order");
        self.run_order(&order, Cancel::pair().1).await
    }

    pub(crate) async fn run_order(&self, order: &Order, cancel: Cancel) -> Report {
        run_parts(order, self.parts(cancel)).await
    }

    /// The parts of a job of this cluster: its gateway and its stores.
    pub(crate) fn parts(&self, cancel: Cancel) -> Parts {
        let stores = &self.stores.stores;
        Parts {
            gateway: self.link.gateway.clone(),
            deployments: stores.full(),
            stateful_sets: stores.full(),
            daemon_sets: stores.full(),
            jobs: stores.full(),
            pods: stores.full(),
            // No operation of the k3s tests is a download.
            downloads: std::env::temp_dir(),
            cancel,
        }
    }

    /// Runs kubectl in the cluster.
    pub(crate) async fn kubectl(&self, arguments: &[&str]) -> String {
        self.k3s.kubectl(arguments).await.unwrap_or_else(|error| panic!("kubectl {arguments:?}: {error}"))
    }

    /// The object as kubectl reads it, or `None` when it is not there.
    pub(crate) async fn object(&self, namespace: &str, kind: Kind, name: &str) -> Option<Json> {
        let resource = resource(kind);
        let arguments = ["get", &resource, name, "--namespace", namespace, "--output", "json", "--ignore-not-found"];
        let text = self.kubectl(&arguments).await;
        (!text.trim().is_empty()).then(|| serde_json::from_str(&text).expect("JSON"))
    }

    /// The names of the objects of a kind in a namespace with this label selector.
    pub(crate) async fn names(&self, namespace: &str, kind: Kind, selector: &str) -> Vec<String> {
        let resource = resource(kind);
        let arguments = ["get", &resource, "--namespace", namespace, "--selector", selector, "--output", "name"];
        let text = self.kubectl(&arguments).await;
        text.lines().filter_map(|line| line.split_once('/')).map(|(_, name)| name.to_owned()).collect()
    }

    /// Asserts that the object of the app and its twin of kubectl have the same state, with
    /// no field that only the identity or the clock gives.
    pub(crate) async fn same_as_kubectl(&self, kind: Kind, name: &str) {
        let app = self.object(APP, kind, name).await.map(normal);
        let kubectl = self.object(KUBECTL, kind, name).await.map(normal);
        assert_eq!(app, kubectl, "{} {name} in {}: the app and kubectl", kind.info().name, self.id);
    }

    /// Waits until a check of the cluster is true. The cluster gives no event to the test.
    pub(crate) async fn until<F: Future<Output = bool>>(&self, what: &str, mut check: impl FnMut() -> F) {
        let end = Instant::now() + LIMIT;
        while !check().await {
            assert!(Instant::now() < end, "in time: {what} in {}", self.id);
            sleep(STEP).await;
        }
    }
}

/// Runs an order with these parts to its end. Gives its last report.
pub(crate) async fn run_parts(order: &Order, parts: Parts) -> Report {
    let mut seen = Seen::default();
    let state = tokio::time::timeout(LIMIT, ops::run(order, parts, &mut seen)).await;
    let state = state.unwrap_or_else(|_| panic!("in time: {order:?}: {:#?}", seen.last));
    assert_eq!(seen.last.state, state, "the last report has the final state");
    seen.last
}

/// The resource of a kind for kubectl, with its group.
pub(crate) fn resource(kind: Kind) -> String {
    let info = kind.info();
    if info.group.is_empty() { info.plural.to_owned() } else { format!("{}.{}", info.plural, info.group) }
}

/// The object with no field of its identity, its versions, its status or its clock. A UID
/// in an owner reference or in a label of a Job is the UID of an other object of the same
/// namespace, so it becomes `uid`.
pub(crate) fn normal(mut object: Json) -> Json {
    let uid = Json::from("uid");
    if let Some(map) = object.as_object_mut() {
        map.remove("status");
    }
    if let Some(metadata) = object.get_mut("metadata").and_then(Json::as_object_mut) {
        for field in ["namespace", "uid", "resourceVersion", "creationTimestamp", "generation", "managedFields"] {
            metadata.remove(field);
        }
    }
    for owner in object.pointer_mut("/metadata/ownerReferences").and_then(Json::as_array_mut).into_iter().flatten() {
        owner["uid"] = uid.clone();
    }
    let time = "kubectl.kubernetes.io/restartedAt";
    if let Some(at) = object.pointer_mut("/spec/template/metadata/annotations").and_then(|map| map.get_mut(time)) {
        *at = Json::from("time");
    }
    for labels in ["/metadata/labels", "/spec/template/metadata/labels", "/spec/selector/matchLabels"] {
        for key in ["controller-uid", "batch.kubernetes.io/controller-uid"] {
            if let Some(value) = object.pointer_mut(labels).and_then(|map| map.get_mut(key)) {
                *value = uid.clone();
            }
        }
    }
    object
}

/// The last report of a job.
#[derive(Default)]
struct Seen {
    last: Report,
}

impl Progress for Seen {
    fn show(&mut self, report: &Report) {
        self.last = report.clone();
    }
}

/// A job that a test can cancel: it gives each report to the test.
pub(crate) struct Watched {
    pub(crate) reports: mpsc::UnboundedReceiver<Report>,
    pub(crate) cancel: watch::Sender<bool>,
}

struct Forward(Option<mpsc::UnboundedSender<Report>>);

impl Progress for Forward {
    fn show(&mut self, report: &Report) {
        if self.0.as_ref().is_some_and(|sender| sender.send(report.clone()).is_err()) {
            // The test stopped to read: no more reports go to it.
            self.0 = None;
        }
    }
}

impl Cluster {
    /// Runs an order with a cancel of the test. Gives the reports and the final state.
    pub(crate) fn watch(&self, order: Order) -> (Watched, impl Future<Output = JobState> + use<>) {
        let (cancel, signal) = Cancel::pair();
        let (sender, reports) = mpsc::unbounded_channel();
        let parts = self.parts(signal);
        let run = async move { ops::run(&order, parts, &mut Forward(Some(sender))).await };
        (Watched { reports, cancel }, run)
    }
}
