//! A k3s cluster behind the fault proxy, its pods, and the app core with its context through
//! the proxy, as from Finder. The core keeps the default settings of the sockets: a ping after
//! 10 s with no frame and 5 s for its answer.

use std::sync::Arc;
use std::time::Duration;

use fresnel_core::app::{Command, Core, CoreParts, CoreSettings, CoreView};
use fresnel_core::cluster::{ClusterId, Link, Registry};
use fresnel_core::journal::{Entry, Journal};
use fresnel_core::net::{NetSettings, WallClock};
use fresnel_core::store::ObjectKey;
use fresnel_core::terminal::{Program, Terminal, TerminalRequest, TerminalView};
use fresnel_testkit::{FaultProxy, Home, K3s};
use k8s_openapi::api::core::v1::Pod;
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::common::{TestTasks, connect, registry, until_live};
use crate::contexts::kubeconfig;

/// The image of each container, as in the other cluster tests.
pub(crate) const BUSYBOX: &str = "rancher/mirrored-library-busybox:1.36.1";
/// The context of the cluster in the kubeconfig of the core.
pub(crate) const CLUSTER: &str = "k3s";
/// The PID 1 of the `shell` pod: a loop that reaps each orphan that reparents to it.
pub(crate) const REAPER: &str = "while true; do wait; sleep 1; done";
/// The longest wait for a state of the core, of a screen or of a pod.
pub(crate) const LIMIT: Duration = Duration::from_secs(120);

/// The fields drop in this sequence: the core stops before the proxy and the cluster.
pub(crate) struct Bed {
    pub(crate) core: Core,
    woken: Arc<Notify>,
    _home: Home,
    /// The API with no proxy, for the changes of the test itself that kubectl cannot make.
    pub(crate) api: Link,
    _registry: Registry,
    _tasks: TestTasks,
    pub(crate) proxy: FaultProxy,
    pub(crate) k3s: K3s,
}

impl Bed {
    pub(crate) async fn start() -> Self {
        let k3s = K3s::start("terminal").await.expect("k3s starts");
        let proxy = FaultProxy::start(k3s.api_addr()).await.expect("a proxy");
        let through = k3s.kubeconfig_through(proxy.addr()).expect("a kubeconfig");
        let home = Home::new("k3s-terminal", &["kubeconfig.yaml"]).expect("a home folder");
        home.write("work/kubeconfig.yaml", &kubeconfig(&[(CLUSTER, through)])).expect("the kubeconfig");
        let images =
            format!("version = 1\n[terminal]\ndebug_image = \"{BUSYBOX}\"\nnode_shell_image = \"{BUSYBOX}\"\n");
        home.write("state/config.toml", &images).expect("the settings");
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
        let tasks = TestTasks::new(&Handle::current());
        let registry = registry(&tasks, &Handle::current(), &k3s.kubeconfig(), NetSettings::default());
        let api = connect(&registry, "default");
        until_live(&api).await;
        let bed = Self { core, woken, _home: home, api, _registry: registry, _tasks: tasks, proxy, k3s };
        let pod = |name: &'static str| ["run", name, "--image", BUSYBOX, "--restart", "Never"];
        // The PID 1 of `shell` reaps: the orphans of a session that the app signalled reparent
        // to it, so a leaked process shows as a live process, not a zombie that no one reaps.
        bed.kubectl(&[&pod("shell")[..], &["--command", "--", "sh", "-c", REAPER]].concat()).await;
        // The main process of `main` is a shell with an input and a terminal, for an attach.
        bed.kubectl(&[&pod("main")[..], &["--stdin", "--tty", "--attach=false", "--command", "--", "sh"]].concat())
            .await;
        bed.kubectl(&["wait", "--for=condition=Ready", "pod/shell", "pod/main", "--timeout=300s"]).await;
        bed
    }

    /// Runs kubectl in the cluster and gives its output.
    pub(crate) async fn kubectl(&self, arguments: &[&str]) -> String {
        self.k3s.kubectl(arguments).await.unwrap_or_else(|error| panic!("kubectl {arguments:?}: {error}"))
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
            let journal: Vec<String> = view.journal.entries.iter().map(|entry| line(entry)).collect();
            let states: Vec<String> =
                view.terminals.iter().map(|view| format!("{:?}", view.terminal.state())).collect();
            panic!("in time: {what}\nterminals: {states:#?}\njournal: {journal:#?}")
        })
    }

    /// The request of a terminal on a pod of the namespace `default`, with the UID of the
    /// pod in the store of the core, as a view takes it from its row.
    pub(crate) async fn request(&self, program: Program, pod: &str, container: &str) -> TerminalRequest {
        let key = ObjectKey::new("default", pod);
        let view = self.until("the pod in the store of the core", |view| uid(view, &key).is_some()).await;
        let uid = uid(&view, &key).unwrap_or_default();
        TerminalRequest { cluster: ClusterId::new(CLUSTER), object: key, uid, container: container.into(), program }
    }

    /// Opens a terminal and gives it when its program runs.
    pub(crate) async fn open(&self, request: TerminalRequest) -> TerminalView {
        let before = self.core.view().terminals.iter().map(|view| view.id).max().unwrap_or(0);
        self.core.send(Command::OpenTerminal(request));
        let view = self.until("a new terminal", |view| view.terminals.iter().any(|view| view.id > before)).await;
        let opened = view.terminals.iter().find(|view| view.id > before).cloned().expect("the new terminal");
        let terminal = opened.terminal.clone();
        self.until("the open session", move |_| {
            let state = terminal.state();
            assert!(!state.is_final(), "the session ended: {state:?}");
            state == fresnel_core::terminal::SessionState::Open
        })
        .await;
        opened
    }

    /// Waits until a row of the screen of the terminal holds the text.
    pub(crate) async fn screen(&self, terminal: &Terminal, text: &str) -> Vec<String> {
        let found = |rows: &[String]| rows.iter().any(|row| row.contains(text));
        let wait = async {
            loop {
                let rows = crate::screen::rows(terminal);
                if found(&rows) {
                    return rows;
                }
                self.woken.notified().await;
            }
        };
        timeout(LIMIT, wait).await.unwrap_or_else(|_| {
            panic!("in time: {text:?} on the screen\n{:#?}\n{:?}", crate::screen::rows(terminal), terminal.state())
        })
    }
}

/// The UID of a pod in the store of the core.
fn uid(view: &CoreView, key: &ObjectKey) -> Option<Arc<str>> {
    let data = view.clusters.iter().find(|row| row.id.as_str() == CLUSTER)?.data.as_ref()?;
    let pod: &Pod = data.pods.items.get(key)?;
    Some(pod.metadata.uid.as_deref()?.into())
}

/// One entry of the journal as one text.
pub(crate) fn line(entry: &Entry) -> String {
    format!("{:?} {}: {}", entry.level, entry.source, entry.text)
}

/// Waits until a condition that no wake of the core tells is met, such as a state of the pod
/// that kubectl reads: the check runs each 100 ms.
pub(crate) async fn eventually<F: Future<Output = bool>>(what: &str, met: impl Fn() -> F) {
    let wait = async {
        while !met().await {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    timeout(LIMIT, wait).await.unwrap_or_else(|_| panic!("in time: {what}"));
}
