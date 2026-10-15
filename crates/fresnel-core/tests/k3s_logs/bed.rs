//! The cluster of the test and the app before it: the stores and the owner index on
//! connections through a fault proxy, and the log views on the same connections.

use std::sync::Arc;
use std::time::Duration;

use fresnel_core::cluster::{ClusterId, Link, Registry};
use fresnel_core::index::ClusterIndexes;
use fresnel_core::kinds::Kind;
use fresnel_core::logs::{LineKind, LogLine, LogOptions, LogRequest, LogSession, LogSettings, LogView, SessionParts};
use fresnel_core::net::task::Backoff;
use fresnel_core::net::{NetSettings, WallClock};
use fresnel_core::published::{Changes, Disposal, Held, Notice, Reader};
use fresnel_core::store::{ObjectKey, Snapshot, StoreSettings};
use fresnel_testkit::{FaultProxy, K3s};
use k8s_openapi::api::core::v1::Pod;
use tokio::runtime::Handle;
use tokio::time::timeout;

use crate::common::{TestTasks, connect, registry, until_live};
use crate::store_bed::Stores;

/// The namespace of the pods of the test.
pub(crate) const NAMESPACE: &str = "default";
/// The image of each container. The scene of the index test uses it too.
pub(crate) const BUSYBOX: &str = "rancher/mirrored-library-busybox:1.36.1";
/// The longest wait for a state of the cluster: an image pull and the start of pods.
pub(crate) const LIMIT: Duration = Duration::from_secs(180);

/// The fields drop in this sequence: the views and the stores go before the connections,
/// and the connections before the cluster.
pub(crate) struct Bed {
    pub(crate) indexes: ClusterIndexes,
    pub(crate) stores: Stores,
    disposal: Disposal,
    pub(crate) link: Link,
    _registry: Registry,
    pub(crate) proxy: FaultProxy,
    pub(crate) tasks: TestTasks,
    pub(crate) k3s: K3s,
}

impl Bed {
    pub(crate) async fn start() -> Self {
        let k3s = K3s::start("m9-logs").await.expect("k3s starts");
        let tasks = TestTasks::new(&Handle::current());
        let proxy = FaultProxy::start(k3s.api_addr()).await.expect("a proxy");
        let kubeconfig = k3s.kubeconfig_through(proxy.addr()).expect("a kubeconfig");
        let registry = registry(&tasks, &Handle::current(), &kubeconfig, NetSettings::default());
        let link = connect(&registry, "default");
        until_live(&link).await;
        let stores = Stores::start(&tasks, &link, StoreSettings::default());
        stores.until_live(LIMIT).await;
        let indexes = stores.indexes(&tasks);
        let disposal = Disposal::start(&tasks.tasks);
        Self { indexes, stores, disposal, link, _registry: registry, proxy, tasks, k3s }
    }

    /// Runs kubectl in the cluster and gives its output.
    pub(crate) async fn kubectl(&self, arguments: &[&str]) -> String {
        self.k3s.kubectl(arguments).await.unwrap_or_else(|error| panic!("kubectl {arguments:?}: {error}"))
    }

    /// The UID of an object of the namespace of the test.
    pub(crate) async fn uid(&self, kind: &str, name: &str) -> String {
        self.kubectl(&["get", kind, name, "--namespace", NAMESPACE, "--output", "jsonpath={.metadata.uid}"]).await
    }

    fn pods(&self) -> Reader<Snapshot<Pod>> {
        self.stores.stores.full::<Pod>().expect("the store of the pods")
    }

    /// Waits until the store of the app has a pod of this name that meets the condition.
    pub(crate) async fn until_pod(&self, name: &str, what: &str, met: impl Fn(&Pod) -> bool) -> Arc<Pod> {
        let mut pods = self.pods();
        let key = ObjectKey::new(NAMESPACE, name);
        let wait = async {
            loop {
                if let Some(pod) = pods.read().items.get(&key).filter(|pod| met(pod)) {
                    return Arc::clone(pod);
                }
                pods.changed().await;
            }
        };
        timeout(LIMIT, wait).await.unwrap_or_else(|_| panic!("in time: the pod {name}: {what}"))
    }

    /// A log view of an object of the namespace of the test, on the connections of the app.
    pub(crate) fn view(&self, kind: Kind, name: &str, uid: &str, options: LogOptions) -> Viewer {
        let notice = Notice::new();
        let parts = SessionParts {
            tasks: self.tasks.tasks.clone(),
            pods: self.pods(),
            owners: self.indexes.owners(),
            signal: self.link.net.signal(),
            clock: WallClock::system(),
            settings: LogSettings::default(),
            restart: Backoff::default(),
            bin: self.disposal.bin(),
            notice: notice.clone(),
        };
        let request = LogRequest {
            cluster: ClusterId::new("default"),
            kind,
            key: ObjectKey::new(NAMESPACE, name),
            uid: uid.into(),
            options,
        };
        Viewer { changes: notice.changes(), session: LogSession::start(request, parts, self.link.gateway.clone()) }
    }
}

/// One log view of the test.
pub(crate) struct Viewer {
    pub(crate) session: LogSession,
    changes: Changes,
}

impl Viewer {
    /// Waits until the view meets the condition, and gives that view.
    pub(crate) async fn until(&mut self, what: &str, mut met: impl FnMut(&LogView) -> bool) -> Held<LogView> {
        let wait = async {
            loop {
                let view = self.session.view();
                if met(&view) {
                    return view;
                }
                self.changes.next().await;
            }
        };
        let view = timeout(LIMIT, wait).await;
        view.unwrap_or_else(|_| panic!("in time: {what}: {:#?}", sources(&self.session.view())))
    }
}

/// Each source of a view with its state and its counts, for the message of a test.
pub(crate) fn sources(view: &LogView) -> Vec<String> {
    let line = |source: &fresnel_core::logs::SourceView| {
        format!(
            "{}/{}: {} ({} lines, {} again)",
            source.pod.name, source.container, source.state, source.lines, source.repeated
        )
    };
    view.sources.iter().map(line).collect()
}

/// Each line of a view with the pod of its source: a marker has the mark `marker:`.
pub(crate) fn lines(view: &LogView) -> Vec<(String, String)> {
    let lines = &view.lines;
    let text = |line: &LogLine| {
        let pod = view.sources.get(line.source as usize).map(|source| source.pod.name.to_string()).unwrap_or_default();
        let text = match line.kind {
            LineKind::Log => line.text.to_string(),
            LineKind::Marker => format!("marker: {}", line.text),
        };
        (pod, text)
    };
    lines.range(lines.first(), lines.end()).map(|(_, line)| text(line)).collect()
}
