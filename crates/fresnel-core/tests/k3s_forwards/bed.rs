//! The cluster of the test with its scene, the fault proxy before it, and the app core on
//! the connections through the proxy, started as from Finder.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use fresnel_core::app::{Command, Core, CoreParts, CoreSettings, CoreView, ForwardRow};
use fresnel_core::cluster::{ClusterId, Gateway};
use fresnel_core::forward::{Bind, ForwardRule, LocalPort};
use fresnel_core::journal::Journal;
use fresnel_core::kinds::Kind;
use fresnel_core::net::WallClock;
use fresnel_core::net::detect::{Detector, Disturbance};
use fresnel_core::store::ObjectKey;
use fresnel_testkit::{FaultProxy, Home, K3s};
use kube::core::Request;
use kube::core::params::PostParams;
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::common::{TestTasks, connect, registry, until_live};
use crate::contexts::kubeconfig;

const SCENE: &str = include_str!("scene.yaml");
pub(crate) const NAMESPACE: &str = "forwards";
/// The longest wait for a state of the core.
pub(crate) const LIMIT: Duration = Duration::from_secs(180);
/// The images of the scene come from the network at the first start.
const AT_REST: [&[&str]; 7] = [
    &["wait", "--for=condition=Ready", "pod/web", "pod/slow", "--timeout=300s"],
    &["rollout", "status", "deployment/deploy", "--timeout=300s"],
    &["rollout", "status", "statefulset/stateful", "--timeout=300s"],
    &["rollout", "status", "daemonset/daemon", "--timeout=300s"],
    &["wait", "--for=jsonpath={.status.readyReplicas}=1", "replicaset/replicas", "--timeout=300s"],
    &["wait", "--for=jsonpath={.status.readyReplicas}=1", "replicationcontroller/controller", "--timeout=300s"],
    &["wait", "--for=jsonpath={.status.ready}=1", "job/job", "--timeout=300s"],
];

/// A detector that sees a wake when the test sets its flag.
struct OnWake(Arc<AtomicBool>);

impl Detector for OnWake {
    fn look(&mut self) -> Option<Disturbance> {
        self.0.swap(false, Ordering::Relaxed).then_some(Disturbance::Wake(Duration::from_secs(60)))
    }
}

/// The fields drop in this sequence: the core goes before the proxy, the proxy before k3s.
pub(crate) struct Bed {
    pub(crate) core: Core,
    woken: Arc<Notify>,
    pub(crate) woke: Arc<AtomicBool>,
    _home: Home,
    pub(crate) proxy: FaultProxy,
    _setup: (fresnel_core::cluster::Link, fresnel_core::cluster::Registry, TestTasks),
    pub(crate) k3s: K3s,
}

impl Bed {
    pub(crate) async fn start() -> Self {
        let k3s = K3s::start("m5-forwards").await.expect("k3s starts");
        let tasks = TestTasks::new(&Handle::current());
        let setup = registry(&tasks, &Handle::current(), &k3s.kubeconfig(), Default::default());
        let admin = connect(&setup, "default");
        until_live(&admin).await;
        create_each(&admin.gateway, SCENE).await;
        for step in AT_REST {
            let arguments = [&["--namespace", NAMESPACE], step].concat();
            k3s.kubectl(&arguments).await.unwrap_or_else(|error| panic!("kubectl {step:?}: {error}"));
        }
        let proxy = FaultProxy::start(k3s.api_addr()).await.expect("a proxy");
        let through = k3s.kubeconfig_through(proxy.addr()).expect("a kubeconfig");
        let home = Home::new("k3s-forwards", &["kubeconfig.yaml"]).expect("a home folder");
        home.write("work/kubeconfig.yaml", &kubeconfig(&[("k3s", through)])).expect("the kubeconfig");
        let woke = Arc::new(AtomicBool::new(false));
        let settings = CoreSettings { detect_period: Duration::from_millis(20), ..CoreSettings::default() };
        let core = Core::new(CoreParts {
            runtime: Handle::current(),
            base_env: home.finder_env(),
            journal: Journal::new(10_000, WallClock::system()),
            state_file: home.path("state/state.json"),
            config_file: home.path("state/config.toml"),
            downloads: home.path("Downloads"),
            detectors: vec![Box::new(OnWake(Arc::clone(&woke)))],
            settings,
        });
        let woken = Arc::new(Notify::new());
        core.start({
            let woken = Arc::clone(&woken);
            move || woken.notify_one()
        });
        Self { core, woken, woke, _home: home, proxy, _setup: (admin, setup, tasks), k3s }
    }

    /// Waits until the view of the core meets the condition.
    pub(crate) async fn until(&self, what: &str, met: impl Fn(&CoreView) -> bool) -> CoreView {
        let wait = async {
            loop {
                let view = self.core.view();
                if met(&view) {
                    return view;
                }
                // The stores ask for a repaint at each change; a wait with no wake ends after
                // a second, so a time alone also gets its look.
                match timeout(Duration::from_secs(1), self.woken.notified()).await {
                    Ok(()) | Err(_) => {}
                }
            }
        };
        timeout(LIMIT, wait).await.unwrap_or_else(|_| panic!("in time: {what}: {:#?}", self.core.view().forwards))
    }

    /// Starts a forward of `rule` and gives its local address once it is active.
    pub(crate) async fn active(&self, rule: &ForwardRule) -> SocketAddr {
        self.core.send(Command::StartForward(rule.clone()));
        let active = |row: &ForwardRow| row.view.state.name() == "active" && row.view.address().is_some();
        let view = self.until(&format!("{rule} is active"), |view| forward(view, rule).is_some_and(active)).await;
        forward(&view, rule).and_then(|row| row.view.address()).expect("an address")
    }

    pub(crate) async fn kubectl(&self, arguments: &[&str]) -> String {
        let arguments = [&["--namespace", NAMESPACE], arguments].concat();
        self.k3s.kubectl(&arguments).await.unwrap_or_else(|error| panic!("kubectl {arguments:?}: {error}"))
    }
}

/// A rule of the cluster `k3s` to an object of the scene, on an automatic port.
pub(crate) fn rule(kind: Kind, name: &str, port: u16) -> ForwardRule {
    ForwardRule {
        cluster: ClusterId::new("k3s"),
        kind,
        key: ObjectKey::new(NAMESPACE, name),
        port,
        local: LocalPort::Automatic,
        bind: Bind::Localhost,
    }
}

pub(crate) fn forward<'a>(view: &'a CoreView, rule: &ForwardRule) -> Option<&'a ForwardRow> {
    view.forwards.iter().find(|row| &row.view.rule == rule)
}

/// The page of the pod behind a local address: the word of its object, or why none came.
/// The read ends with the line of the page, not with the end of the data: containerd ends a
/// forwarded connection 1 s after the pod (port forward prototype, problem 6).
pub(crate) async fn page(address: SocketAddr) -> Result<String, String> {
    let fetch = async {
        let mut client = TcpStream::connect(address).await.map_err(|error| format!("connect: {error}"))?;
        client.write_all(b"GET / HTTP/1.0\r\n\r\n").await.map_err(|error| format!("write: {error}"))?;
        let (mut answer, mut buffer) = (String::new(), [0_u8; 4096]);
        loop {
            let read = client.read(&mut buffer).await.map_err(|error| format!("read: {error}"))?;
            answer.push_str(&String::from_utf8_lossy(buffer.get(..read).unwrap_or_default()));
            let body = answer.split_once("\r\n\r\n").map(|(_, body)| body);
            match body.filter(|body| body.ends_with('\n')) {
                Some(body) if answer.starts_with("HTTP/1.") && answer.contains(" 200 ") => {
                    return Ok(body.trim().to_owned());
                }
                Some(_) => return Err(format!("an answer: {answer}")),
                None if read == 0 => return Err(format!("the end of the data after: {answer}")),
                None => {}
            }
        }
    };
    timeout(Duration::from_secs(20), fetch).await.unwrap_or_else(|_| Err("no answer in 20 s".to_owned()))
}

/// Creates each object of a manifest through the connections of the setup.
async fn create_each(api: &Gateway, manifest: &str) {
    let documents: Vec<Value> = serde_saphyr::from_multiple(manifest).expect("the manifest is YAML");
    for document in documents {
        let text = |pointer: &str| document.pointer(pointer).and_then(Value::as_str);
        let (kind, name) = (text("/kind").expect("a kind"), text("/metadata/name").expect("a name"));
        let info = Kind::named(kind).expect("a kind of the app").info();
        let request =
            Request::new(text("/metadata/namespace").map_or_else(|| info.path(), |namespace| info.path_in(namespace)));
        let body = serde_json::to_vec(&document).expect("JSON");
        let create = request.create(&PostParams::default(), body).expect("a request");
        let _created: Value = api.send(create).await.unwrap_or_else(|error| panic!("create {kind} {name}: {error}"));
    }
}
