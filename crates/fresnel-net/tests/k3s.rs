//! The connection layer on a real kube-apiserver: k3s in Docker behind the fault proxy.
//!
//! The steps are in one test, because the start of a cluster is the slow part.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
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
