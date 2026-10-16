//! The cluster of the test, its pods, and the app before it: the connections of the app
//! through a fault proxy, with pings after 1 s and a limit of 1 s for their answer.

use std::time::Duration;

use fresnel_core::cluster::{Link, Registry};
use fresnel_core::journal::Journal;
use fresnel_core::net::{NetSettings, SocketSettings, UpgradeSettings, WallClock};
use fresnel_core::store::ObjectKey;
use fresnel_core::stream::{ExecSession, Output};
use fresnel_testkit::{FaultProxy, K3s};
use tokio::runtime::Handle;

use crate::common::{TestTasks, connect, in_time, registry, until_live};

/// The image of each container, as in the other cluster tests.
const BUSYBOX: &str = "rancher/mirrored-library-busybox:1.36.1";
/// A web server with one page on port 8080.
const WEB: &str = "mkdir -p /www && echo fresnel-forward > /www/index.html && exec httpd -f -p 8080 -h /www";

/// The fields drop in this sequence: the connections go before the cluster.
pub(crate) struct Bed {
    pub(crate) link: Link,
    _registry: Registry,
    pub(crate) proxy: FaultProxy,
    _tasks: TestTasks,
    k3s: K3s,
}

impl Bed {
    pub(crate) async fn start() -> Self {
        let k3s = K3s::start("m5-streams").await.expect("k3s starts");
        let tasks = TestTasks::new(&Handle::current());
        let proxy = FaultProxy::start(k3s.api_addr()).await.expect("a proxy");
        let kubeconfig = k3s.kubeconfig_through(proxy.addr()).expect("a kubeconfig");
        let registry = registry(&tasks, &Handle::current(), &kubeconfig, settings());
        let link = connect(&registry, "default");
        until_live(&link).await;
        let bed = Self { link, _registry: registry, proxy, _tasks: tasks, k3s };
        bed.kubectl(&["run", "shell", "--image", BUSYBOX, "--restart", "Never", "--command", "--", "sleep", "3600"])
            .await;
        bed.kubectl(&["run", "web", "--image", BUSYBOX, "--restart", "Never", "--command", "--", "sh", "-c", WEB])
            .await;
        bed.kubectl(&["wait", "--for=condition=Ready", "pod/shell", "pod/web", "--timeout=300s"]).await;
        bed
    }

    /// Runs kubectl in the cluster and gives its output.
    pub(crate) async fn kubectl(&self, arguments: &[&str]) -> String {
        self.k3s.kubectl(arguments).await.unwrap_or_else(|error| panic!("kubectl {arguments:?}: {error}"))
    }
}

/// Short pings, so that a silent connection breaks in about 2 s.
fn settings() -> NetSettings {
    let socket = SocketSettings {
        ping_after: Duration::from_secs(1),
        answer_limit: Duration::from_secs(1),
        close_limit: Duration::from_secs(1),
        ..SocketSettings::default()
    };
    NetSettings { upgrade: UpgradeSettings { socket, ..UpgradeSettings::default() }, ..NetSettings::default() }
}

/// A pod of the namespace `default`.
pub(crate) fn pod(name: &str) -> ObjectKey {
    ObjectKey::new("default", name)
}

/// The arguments of `sh -c` with this script.
pub(crate) fn sh(script: &str) -> Option<Vec<String>> {
    Some(["sh", "-c", script].map(str::to_owned).to_vec())
}

pub(crate) fn journal() -> Journal {
    Journal::new(100, WallClock::system())
}

/// The output of a session to its end: the standard output and the error output.
pub(crate) async fn output(session: &mut ExecSession) -> (Vec<u8>, Vec<u8>) {
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    while let Some(output) = in_time("the output", session.next()).await {
        match output {
            Output::Stdout(data) => stdout.extend_from_slice(&data),
            Output::Stderr(data) => stderr.extend_from_slice(&data),
        }
    }
    (stdout, stderr)
}
