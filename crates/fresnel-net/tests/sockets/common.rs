//! The helpers of the socket tests: a cluster of the app on the fake API server.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use fresnel_net::credential::PluginSpec;
use fresnel_net::env::ProcessEnv;
use fresnel_net::task::Tasks;
use fresnel_net::{
    ChannelState, ChannelStatus, Cluster, ClusterSpec, ClusterStatus, Endpoint, GroupStatus, NetError, NetSettings,
    RequestKind, ResponseBody, WallClock,
};
use fresnel_testkit::FakeApiServer;
use http::{Request, Response, StatusCode};
use http_body_util::BodyExt as _;
use rustls::pki_types::CertificateDer;
use rustls::{ClientConfig, RootCertStore};
use tokio::runtime::Handle;
use tokio::time::timeout;

/// The longest wait for a signal that must arrive.
pub(crate) const LIMIT: Duration = Duration::from_secs(30);

pub(crate) async fn in_time<T>(what: &str, signal: impl Future<Output = T>) -> T {
    timeout(LIMIT, signal).await.unwrap_or_else(|_| panic!("in time: {what}"))
}

/// The tasks of the clusters of one test. A task that ends is a defect, and the drop of
/// this value fails the test for it.
pub(crate) struct TestTasks {
    pub(crate) tasks: Tasks,
    ended: Arc<Mutex<Vec<String>>>,
}

impl TestTasks {
    pub(crate) fn new() -> Self {
        let ended = Arc::new(Mutex::new(Vec::new()));
        let reports = Arc::clone(&ended);
        let tasks = Tasks::new(Handle::current(), move |report: fresnel_net::task::TaskReport| {
            reports.lock().expect("a lock").push(report.to_string());
        });
        Self { tasks, ended }
    }
}

impl Drop for TestTasks {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert_eq!(*self.ended.lock().expect("a lock"), Vec::<String>::new(), "a task of the app ended");
        }
    }
}

/// The TLS settings of a client that trusts the certificate of the fake server.
pub(crate) fn trust(server: &FakeApiServer) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(server.certificate_der().to_vec())).expect("a certificate");
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder =
        ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().expect("versions");
    Arc::new(builder.with_root_certificates(roots).with_no_client_auth())
}

/// The spec of a cluster whose API server is at `address`: the fake server or a proxy before it.
pub(crate) fn spec(name: &str, address: SocketAddr, tls: Arc<ClientConfig>) -> ClusterSpec {
    ClusterSpec {
        name: name.to_owned(),
        endpoint: Endpoint {
            host: address.ip().to_string(),
            port: address.port(),
            base_path: String::new(),
            tls_server_name: None,
            tls,
        },
        plugin: None,
        env: ProcessEnv::default(),
        settings: NetSettings::default(),
        clock: WallClock::system(),
    }
}

/// Waits until the status of the cluster meets the condition.
pub(crate) async fn until(cluster: &Cluster, what: &str, condition: impl Fn(&ClusterStatus) -> bool) -> ClusterStatus {
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

/// The first channel of each purpose: watches, requests, logs.
pub(crate) fn first_channels(status: &ClusterStatus) -> [&ChannelStatus; 3] {
    [&status.watches.channels[0], &status.requests.channels[0], &status.logs.channels[0]]
}

pub(crate) fn all_live(group: &GroupStatus) -> bool {
    group.channels.iter().all(|channel| channel.state == ChannelState::Live)
}

/// Waits until each channel of the cluster is live.
pub(crate) async fn until_live(cluster: &Cluster) -> ClusterStatus {
    until(cluster, "each channel is live", |status| {
        [&status.watches, &status.requests, &status.logs].into_iter().all(all_live)
    })
    .await
}

pub(crate) async fn send(cluster: &Cluster, kind: RequestKind, path: &str) -> Result<Response<ResponseBody>, NetError> {
    let mut request = Request::get(path).body(Bytes::new()).expect("a request");
    request.extensions_mut().insert(kind);
    cluster.send(request).await
}

/// Sends a request and reads its full answer.
pub(crate) async fn fetch(cluster: &Cluster, kind: RequestKind, path: &str) -> Result<(StatusCode, String), NetError> {
    let (head, body) = send(cluster, kind, path).await?.into_parts();
    let text = body.collect().await?.to_bytes();
    Ok((head.status, String::from_utf8(text.to_vec()).expect("text")))
}

/// A folder for the files of one test. Its drop removes it.
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    pub(crate) fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!("fresnel-net-test-{label}-{:08x}", fastrand::u32(..)));
        std::fs::create_dir(&folder).expect("a new folder");
        Self(folder)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    /// The text of a file of the folder. Empty when the file is absent.
    pub(crate) fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.0.join(file)).unwrap_or_default()
    }

    pub(crate) fn write(&self, file: &str, text: &str) {
        std::fs::write(self.0.join(file), text).expect("a write to the test folder");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("cannot remove {}: {error}", self.0.display());
        }
    }
}

pub(crate) fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// The spec of the fake plugin. It gives the tokens `<token>-1`, `<token>-2` and so on.
pub(crate) fn plugin(state: &Scratch, token: &str) -> PluginSpec {
    PluginSpec {
        command: fixture("fake-plugin").display().to_string(),
        api_version: Some("client.authentication.k8s.io/v1beta1".to_owned()),
        env: vec![
            ("FAKE_PLUGIN_STATE".to_owned(), state.path().display().to_string()),
            ("FAKE_PLUGIN_TOKEN".to_owned(), token.to_owned()),
        ],
        ..PluginSpec::default()
    }
}

/// The environment of a program from Finder: the plugin needs `PATH` for its own tools.
pub(crate) fn finder_env() -> ProcessEnv {
    [("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")].into_iter().collect()
}

/// True while a process with this id exists.
pub(crate) fn process_exists(id: &str) -> bool {
    let id: i32 = id.trim().parse().expect("a process id");
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(id), None).is_ok()
}
