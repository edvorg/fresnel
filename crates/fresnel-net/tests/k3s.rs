//! The connection layer on a real kube-apiserver: k3s in Docker behind the fault proxy.
//!
//! The steps of the first test are in one test, because the start of a cluster is the
//! slow part. The test with a credential plugin has its own cluster, so the two run at the
//! same time.

// k3s in Docker: the cluster job of CI runs on Linux.
#![cfg(unix)]
#![allow(clippy::expect_used, reason = "a test stops with a panic")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use fresnel_net::credential::{CredentialFailure, CredentialState, PluginSpec};
use fresnel_net::env::ProcessEnv;
use fresnel_net::task::{TaskReport, Tasks};
use fresnel_net::{ChannelState, Cluster, ClusterSpec, ClusterStatus, Endpoint, NetSettings, RequestKind, WallClock};
use fresnel_testkit::{FaultProxy, K3s};
use http::Request;
use http_body_util::BodyExt as _;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore};
use tokio::runtime::Handle;
use tokio::time::timeout;

const LIMIT: Duration = Duration::from_secs(30);

/// One value of the kubeconfig, decoded from its base64 form.
fn decoded(kubeconfig: &str, key: &str) -> Vec<u8> {
    let line = kubeconfig.lines().find_map(|line| line.trim().strip_prefix(key)).expect("the key is in the file");
    STANDARD.decode(line.trim()).expect("base64")
}

/// The TLS settings of the cluster admin of the test cluster, from the kubeconfig of the helper.
fn admin(kubeconfig: &Path) -> Arc<ClientConfig> {
    let text = std::fs::read_to_string(kubeconfig).expect("the kubeconfig of the test cluster");
    let certificates = |key| {
        let pem = decoded(&text, key);
        CertificateDer::pem_slice_iter(&pem).collect::<Result<Vec<_>, _>>().expect("certificates")
    };
    let mut roots = RootCertStore::empty();
    for root in certificates("certificate-authority-data:") {
        roots.add(root).expect("a certificate");
    }
    let key = PrivateKeyDer::from_pem_slice(&decoded(&text, "client-key-data:")).expect("a key");
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder =
        ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().expect("versions");
    let identity = certificates("client-certificate-data:");
    Arc::new(builder.with_root_certificates(roots).with_client_auth_cert(identity, key).expect("a client key"))
}

async fn until(cluster: &Cluster, what: &str, condition: impl Fn(&ClusterStatus) -> bool) -> ClusterStatus {
    let mut changes = cluster.changes();
    let wait = async {
        loop {
            let status = cluster.status();
            if condition(&status) {
                return status;
            }
            changes.next().await;
        }
    };
    timeout(LIMIT, wait).await.unwrap_or_else(|_| panic!("no state `{what}` in time: {:#?}", cluster.status()))
}

fn first_channels(status: &ClusterStatus) -> [&fresnel_net::ChannelStatus; 3] {
    [&status.watches.channels[0], &status.requests.channels[0], &status.logs.channels[0]]
}

async fn until_live(cluster: &Cluster) -> ClusterStatus {
    until(cluster, "each channel is live", |status| {
        first_channels(status).iter().all(|channel| channel.state == ChannelState::Live)
    })
    .await
}

/// Sends a read and gives the status with the full text of the answer.
async fn fetch(cluster: &Cluster, kind: RequestKind, path: &str) -> (u16, String) {
    let mut request = Request::get(path).body(Bytes::new()).expect("a request");
    request.extensions_mut().insert(kind);
    let (head, body) = cluster.send(request).await.expect("an answer").into_parts();
    let text = body.collect().await.expect("the body").to_bytes();
    (head.status.as_u16(), String::from_utf8(text.to_vec()).expect("text"))
}

#[tokio::test]
#[ignore = "needs k3s in Docker"]
async fn the_channels_work_on_a_real_kube_apiserver_through_an_idle_time_a_silence_and_a_restart() {
    let k3s = K3s::start("net").await.expect("k3s starts");
    let proxy = FaultProxy::start(k3s.api_addr()).await.expect("a proxy");
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task of the app ended: {report}"));
    let spec = ClusterSpec {
        name: "k3s".to_owned(),
        endpoint: Endpoint {
            host: proxy.addr().ip().to_string(),
            port: proxy.addr().port(),
            base_path: String::new(),
            tls_server_name: None,
            tls: admin(&k3s.kubeconfig()),
        },
        plugin: None,
        env: ProcessEnv::default(),
        settings: NetSettings::default(),
        clock: WallClock::system(),
    };
    let cluster = Cluster::start(&tasks, spec).expect("a cluster");

    // The limit comes from the server. The supervisor port of k3s would give 250.
    let status = until_live(&cluster).await;
    for channel in first_channels(&status) {
        assert_eq!((channel.server_stream_limit, channel.stream_budget), (Some(100), 70), "{channel:?}");
    }
    let (code, version) = fetch(&cluster, RequestKind::Get, "/version").await;
    assert!(code == 200 && version.contains("v1.33.5+k3s1"), "{code}: {version}");
    let (code, namespaces) = fetch(&cluster, RequestKind::ListPage, "/api/v1/namespaces?limit=500").await;
    assert!(code == 200 && namespaces.contains(r#""name":"kube-system""#), "{code}: {namespaces}");
    // A large answer: the server sends it as gzip, and the caller reads plain JSON.
    let (code, openapi) = fetch(&cluster, RequestKind::Get, "/openapi/v2").await;
    assert!(code == 200 && openapi.len() > 1024 * 1024, "{code}: {} bytes", openapi.len());
    let document: serde_json::Value = serde_json::from_str(&openapi).expect("JSON after the gzip decode");
    assert_eq!(document["swagger"], "2.0");

    // A kube-apiserver closes a connection with no request after 90 s. The light request of
    // an idle channel prevents this, so no channel makes a new connection in 100 s.
    let accepted = proxy.counters().accepted;
    tokio::time::sleep(Duration::from_secs(100)).await;
    let status = until_live(&cluster).await;
    assert_eq!(proxy.counters().accepted, accepted, "a channel connected again");
    for channel in first_channels(&status) {
        assert_eq!((channel.connections, &channel.last_error), (1, &None), "{channel:?}");
        assert!(channel.last_frame_age.expect("a frame") < Duration::from_secs(35), "{channel:?}");
    }

    // A watch, then the silent connection. A change during the fault is in the next watch.
    let watch = "/api/v1/namespaces?watch=true&allowWatchBookmarks=true&timeoutSeconds=300";
    let mut request = Request::get(watch).body(Bytes::new()).expect("a request");
    request.extensions_mut().insert(RequestKind::Watch);
    let mut events = cluster.send(request).await.expect("a watch").into_body();
    let first = timeout(LIMIT, events.frame()).await.expect("an event in time").expect("a frame").expect("data");
    assert!(first.into_data().expect("data").starts_with(br#"{"type":"ADDED""#));

    proxy.silence_open();
    let silent_at = Instant::now();
    k3s.kubectl(&["create", "namespace", "made-in-the-silence"]).await.expect("kubectl works");
    until(&cluster, "each channel lost its connection", |status| {
        first_channels(status).iter().all(|channel| channel.connections >= 2)
    })
    .await;
    let shown_after = silent_at.elapsed();
    assert!(shown_after <= Duration::from_secs(20), "the silent connections showed after {shown_after:?}");
    until_live(&cluster).await;
    let (code, namespaces) = fetch(&cluster, RequestKind::ListPage, "/api/v1/namespaces?limit=500").await;
    assert!(code == 200 && namespaces.contains(r#""name":"made-in-the-silence""#), "{code}: {namespaces}");

    // A restart of the API server. A server that starts refuses connections, or accepts and
    // closes them, so each channel connects many times with a pause that grows. Each channel
    // must be live a short time after the server is ready.
    k3s.restart().await.expect("k3s starts again");
    let ready_at = Instant::now();
    until(&cluster, "each channel is live on a new connection", |status| {
        first_channels(status).iter().all(|channel| channel.state == ChannelState::Live && channel.connections >= 3)
    })
    .await;
    let live_after = ready_at.elapsed();
    assert!(live_after <= Duration::from_secs(5), "the channels were live {live_after:?} after the restart");
    let (code, namespaces) = fetch(&cluster, RequestKind::ListPage, "/api/v1/namespaces?limit=500").await;
    assert!(code == 200 && namespaces.contains(r#""name":"made-in-the-silence""#), "{code}: {namespaces}");
    assert_eq!(proxy.task_failures(), Vec::<String>::new());
}

/// A credential plugin in a folder of the test. Each run needs one second, as a plugin that
/// asks a server, and then gives the token of a file. The drop removes the folder.
struct SlowPlugin(PathBuf);

impl SlowPlugin {
    const SCRIPT: &str = "#!/bin/sh
sleep 1
printf '{\"apiVersion\":\"client.authentication.k8s.io/v1\",\"kind\":\"ExecCredential\",\"status\":{\"token\":\"%s\"}}\\n' \\
    \"$(cat \"$(dirname \"$0\")/token\")\"
";

    fn new(token: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;

        let folder = std::env::temp_dir().join(format!("fresnel-net-test-k3s-plugin-{:08x}", fastrand::u32(..)));
        std::fs::create_dir(&folder).expect("a new folder");
        let plugin = Self(folder);
        std::fs::write(plugin.0.join("token"), token).expect("the token file");
        std::fs::write(plugin.command(), Self::SCRIPT).expect("the plugin file");
        std::fs::set_permissions(plugin.command(), std::fs::Permissions::from_mode(0o700)).expect("a program file");
        plugin
    }

    fn command(&self) -> PathBuf {
        self.0.join("plugin")
    }

    fn spec(&self) -> PluginSpec {
        PluginSpec { command: self.command().display().to_string(), ..PluginSpec::default() }
    }
}

/// The spec of a cluster whose user is the token of the plugin. The TLS settings have the
/// certificate of the server only: the token proves the user.
fn spec_with_plugin(name: &str, k3s: &K3s, proxy: &FaultProxy, plugin: &SlowPlugin) -> ClusterSpec {
    let text = std::fs::read_to_string(k3s.kubeconfig()).expect("the kubeconfig of the test cluster");
    let mut roots = RootCertStore::empty();
    for root in CertificateDer::pem_slice_iter(&decoded(&text, "certificate-authority-data:")) {
        roots.add(root.expect("a certificate")).expect("a root");
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder =
        ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().expect("versions");
    ClusterSpec {
        name: name.to_owned(),
        endpoint: Endpoint {
            host: proxy.addr().ip().to_string(),
            port: proxy.addr().port(),
            base_path: String::new(),
            tls_server_name: None,
            tls: Arc::new(builder.with_root_certificates(roots).with_no_client_auth()),
        },
        plugin: Some(plugin.spec()),
        env: [("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")].into_iter().collect(),
        settings: NetSettings::default(),
        clock: WallClock::system(),
    }
}

impl Drop for SlowPlugin {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("cannot remove {}: {error}", self.0.display());
        }
    }
}

/// A kube-apiserver ends an HTTP/2 connection after its answer to a request with no
/// credential. Thus a channel sends no request before the first token of the plugin, and
/// a cluster with a slow plugin uses one connection for each channel (design rule 3).
#[tokio::test]
#[ignore = "needs k3s in Docker"]
async fn a_channel_keeps_its_first_connection_on_a_real_kube_apiserver_while_a_slow_plugin_runs() {
    let k3s = K3s::start("net-token").await.expect("k3s starts");
    let proxy = FaultProxy::start(k3s.api_addr()).await.expect("a proxy");
    k3s.kubectl(&["create", "serviceaccount", "reader", "--namespace", "default"]).await.expect("a service account");
    let token = k3s.kubectl(&["create", "token", "reader", "--namespace", "default"]).await.expect("a token");
    let plugin = SlowPlugin::new(token.trim());
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task of the app ended: {report}"));
    let cluster = Cluster::start(&tasks, spec_with_plugin("k3s-token", &k3s, &proxy, &plugin)).expect("a cluster");

    until(&cluster, "each channel has its connection and waits for the token", |status| {
        first_channels(status).iter().all(|channel| channel.state == ChannelState::WaitsForToken)
    })
    .await;
    let live = until_live(&cluster).await;
    for channel in first_channels(&live) {
        assert_eq!((channel.connections, &channel.last_error), (1, &None), "{channel:?}");
        assert_eq!(channel.server_stream_limit, Some(100), "the address is the kube-apiserver");
    }
    assert_eq!(live.credential.expect("a credential").plugin_runs, 1);
    // The server knows the user of the token: the service account can read no namespace.
    let (code, version) = fetch(&cluster, RequestKind::Get, "/version").await;
    assert!(code == 200 && version.contains("v1.33.5+k3s1"), "{code}: {version}");
    let (code, refusal) = fetch(&cluster, RequestKind::ListPage, "/api/v1/namespaces?limit=500").await;
    assert!(code == 403 && refusal.contains("system:serviceaccount:default:reader"), "{code}: {refusal}");
    // A connection that the server ended would show here some seconds later at most.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let status = until_live(&cluster).await;
    assert!(first_channels(&status).iter().all(|channel| channel.connections == 1), "{status:#?}");
    assert_eq!(proxy.counters().accepted, 3, "one connection for each channel");
    assert_eq!(proxy.task_failures(), Vec::<String>::new());
}

/// A kube-apiserver also ends an HTTP/2 connection after HTTP 401 to a token that it
/// refuses. With no request of a caller, only the light request of a channel can tell the
/// credential. Then no channel sends a request, and the cluster shows the cause (design
/// rules 3 and 5). With no report, such a token cost 51 connections in 30 s, with no end,
/// and the credential showed as valid.
#[tokio::test]
#[ignore = "needs k3s in Docker"]
async fn a_token_that_a_real_kube_apiserver_refuses_shows_as_refused_and_costs_no_connection_in_a_pause() {
    let k3s = K3s::start("net-refused").await.expect("k3s starts");
    let proxy = FaultProxy::start(k3s.api_addr()).await.expect("a proxy");
    let plugin = SlowPlugin::new("not-a-token-of-this-cluster");
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task of the app ended: {report}"));
    let cluster = Cluster::start(&tasks, spec_with_plugin("k3s-refused", &k3s, &proxy, &plugin)).expect("a cluster");

    // The count of failures in sequence of a credential that the server refused.
    let refusals = |status: &ClusterStatus| match status.credential.as_ref().map(|credential| &credential.state) {
        Some(CredentialState::Failed { failure: CredentialFailure::Refused, failures, .. }) => *failures,
        _ => 0,
    };
    // Measured: the first refusal shows after 2.4 s, of which the plugin runs for 2 s.
    let refused = until(&cluster, "a refused credential", |status| refusals(status) == 1).await;
    let credential = refused.credential.expect("a credential");
    // A proof that goes out on a connection that the server ended a moment before gets no
    // answer, and the next HTTP 401 starts a new proof.
    assert!(credential.plugin_runs == 2 && credential.proofs >= 1, "one more plugin run, then a proof: {credential:?}");

    // The plugin runs again after 2 s and after 4 s more. After the third refusal, the
    // next run is 8 s away, and each channel waits for it on an open connection.
    until(&cluster, "the third refusal", |status| refusals(status) == 3).await;
    until(&cluster, "each channel waits for a token", |status| {
        first_channels(status).iter().all(|channel| channel.state == ChannelState::WaitsForToken)
    })
    .await;
    let accepted = proxy.counters().accepted;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(proxy.counters().accepted, accepted, "a channel connected again while the cluster has no token");
    assert_eq!(cluster.status().credential.expect("a credential").plugin_runs, 4, "one run for each refusal");
    assert_eq!(proxy.task_failures(), Vec::<String>::new());
}
