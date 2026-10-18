//! The port forwards of the core (plan section 12.3) on the fake API server behind a fault
//! proxy. The fake pod sends each byte of each port back, so a local client reads what it
//! wrote. The acceptance of plan section 15 for M5 that the fake server can prove is here;
//! a pod that restarts and an upload of 200 MB to a pod that reads slowly are in `k3s_app`.
//!
//! - `flow`: the bytes of a Pod and of a Service, a new pod, a slow pod, a cluster that is
//!   not selected. `breaks`: a network that goes away and comes back, a wake, six forwards.
//! - `commands`: a port in use, stop, start again, remove, "retry now", the saved rules.
//! - `stalls`: opens with no answer, tunnels that open and fail, the faults of the soak with
//!   a client of each second, and the count of a gap with no visible state.

mod breaks;
mod commands;
mod flow;
mod stalls;

use std::net::SocketAddr;
use std::time::Duration;

use fresnel_core::app::{Command, CoreSettings, CoreView, ForwardRow};
use fresnel_core::cluster::ClusterId;
use fresnel_core::forward::{Bind, ForwardRule, ForwardView, LocalPort};
use fresnel_core::kinds::Kind;
use fresnel_core::metrics::MetricsSettings;
use fresnel_core::net::UpgradeStatus;
use fresnel_core::store::ObjectKey;
use fresnel_testkit::{FakeApiServer, FaultProxy, Home};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::time::timeout;

use super::{App, home, write_kubeconfig};

const SECOND: Duration = Duration::from_secs(1);
const PODS: &str = "/api/v1/pods";
const SERVICES: &str = "/api/v1/services";
const SLICES: &str = "/apis/discovery.k8s.io/v1/endpointslices";

/// A pod of the namespace `shop` that runs and is ready, with the container port 8080 `http`.
fn pod(name: &str, uid: &str) -> String {
    format!(
        r#"{{"metadata":{{"name":"{name}","namespace":"shop","uid":"{uid}","resourceVersion":"9","labels":{{"app":"web"}}}},
        "spec":{{"containers":[{{"name":"app","ports":[{{"name":"http","containerPort":8080}}]}}]}},
        "status":{{"phase":"Running","conditions":[{{"type":"Ready","status":"True"}}]}}}}"#
    )
}

fn list(kind: &str, items: &[String]) -> String {
    format!(
        r#"{{"kind":"{kind}","apiVersion":"v1","metadata":{{"resourceVersion":"10"}},"items":[{}]}}"#,
        items.join(",")
    )
}

/// The Service `web` with the port 80 to the port `http` of its pods, and its EndpointSlice
/// with the pod `web-1`.
fn service_bodies(server: &FakeApiServer) {
    let service = r#"{"metadata":{"name":"web","namespace":"shop","uid":"s1","resourceVersion":"3"},
        "spec":{"selector":{"app":"web"},"ports":[{"name":"http","port":80,"targetPort":"http","protocol":"TCP"}]}}"#;
    server.set_body(SERVICES, &list("ServiceList", &[service.to_owned()]));
    let slice = r#"{"metadata":{"name":"web-x1","namespace":"shop","uid":"e1","resourceVersion":"4",
        "labels":{"kubernetes.io/service-name":"web"}},"addressType":"IPv4",
        "endpoints":[{"addresses":["10.0.0.1"],"conditions":{"ready":true},
        "targetRef":{"kind":"Pod","name":"web-1","namespace":"shop","uid":"u1"}}],
        "ports":[{"name":"http","port":8080,"protocol":"TCP"}]}"#;
    server.set_body(SLICES, &list("EndpointSliceList", &[slice.to_owned()]));
}

/// The core with the cluster `ams` behind a fault proxy. The pod `web-1` runs.
struct Bed {
    app: App,
    server: FakeApiServer,
    proxy: FaultProxy,
    _home: Home,
}

impl Bed {
    async fn start(label: &str) -> Self {
        Self::start_with(label, settings()).await
    }

    async fn start_with(label: &str, settings: CoreSettings) -> Self {
        let server = FakeApiServer::start().await.expect("a server");
        server.set_body(PODS, &list("PodList", &[pod("web-1", "u1")]));
        service_bodies(&server);
        let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
        let home = home(label);
        write_kubeconfig(&home, "ams", &[("ams", proxy.addr(), &server, false)]);
        let app = App::started(&home, settings);
        Self { app, server, proxy, _home: home }
    }

    fn start_forward(&self, rule: &ForwardRule) {
        self.app.core.send(Command::StartForward(rule.clone()));
    }

    /// Waits until the forward of `rule` meets the condition.
    async fn until(&self, rule: &ForwardRule, what: &str, met: impl Fn(&ForwardView) -> bool) -> ForwardRow {
        let view = self.app.until(what, |view| forward(view, rule).is_some_and(|row| met(&row.view))).await;
        forward(&view, rule).cloned().expect("the forward")
    }

    /// Waits until the forward is active, and gives its local address.
    async fn active(&self, rule: &ForwardRule) -> SocketAddr {
        let row = self.until(rule, "an active forward", |view| view.state.name() == "active").await;
        row.view.address().expect("a local address")
    }
}

/// Settings of a test: a poll of the metrics each hour, so that only the views wake a wait.
fn settings() -> CoreSettings {
    CoreSettings {
        metrics: MetricsSettings { period: 3600 * SECOND, ..MetricsSettings::default() },
        ..Default::default()
    }
}

/// A rule of the cluster `ams` to an object of the namespace `shop`, on an automatic port.
fn rule(kind: Kind, name: &str, port: u16) -> ForwardRule {
    ForwardRule {
        cluster: ClusterId::new("ams"),
        kind,
        key: ObjectKey::new("shop", name),
        port,
        local: LocalPort::Automatic,
        bind: Bind::Localhost,
    }
}

fn forward<'a>(view: &'a CoreView, rule: &ForwardRule) -> Option<&'a ForwardRow> {
    view.forwards.iter().find(|row| &row.view.rule == rule)
}

/// The upgrade client of the cluster `ams`. A raise of its signal, which tells each channel
/// to test its connection now, drops the warm connections: `dropped` counts them.
fn upgrade(view: &CoreView) -> UpgradeStatus {
    super::row(view, "ams").status().map(|status| status.upgrade.clone()).unwrap_or_default()
}

/// Sends `data` through the local port and reads it back from the pod. Gives the error of
/// the first step that failed.
async fn echo(address: SocketAddr, data: &[u8]) -> Result<(), String> {
    let exchange = async {
        let mut client = TcpStream::connect(address).await.map_err(|error| format!("connect: {error}"))?;
        client.write_all(data).await.map_err(|error| format!("write: {error}"))?;
        let mut back = vec![0_u8; data.len()];
        client.read_exact(&mut back).await.map_err(|error| format!("read: {error}"))?;
        if back == data { Ok(()) } else { Err("other bytes came back".to_owned()) }
    };
    timeout(30 * SECOND, exchange).await.unwrap_or_else(|_| Err("no answer in 30 s".to_owned()))
}

/// How a local client saw its connection end, with no byte of its own.
async fn end_seen(address: SocketAddr) -> Result<&'static str, String> {
    let mut client = TcpStream::connect(address).await.map_err(|error| format!("the port is closed: {error}"))?;
    let mut buffer = [0_u8; 16];
    Ok(match timeout(30 * SECOND, client.read(&mut buffer)).await {
        Ok(Ok(0)) => "end of data",
        Ok(Ok(_)) => "data",
        Ok(Err(_)) => "reset",
        Err(_) => "nothing in 30 s",
    })
}
