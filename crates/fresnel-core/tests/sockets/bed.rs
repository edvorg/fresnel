//! A cluster of the app on the fake API server, through a kubeconfig file.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fresnel_core::cluster::{Link, Registry};
use fresnel_core::net::NetSettings;
use fresnel_testkit::FakeApiServer;
use http::Request;
use tokio::runtime::Handle;

use crate::common::{TestTasks, connect, registry, until_live};

/// The name of the one context of a bed.
pub(crate) const CONTEXT: &str = "bed";

/// One connected cluster. The fields drop in this sequence: the connections go first.
pub(crate) struct Bed {
    pub(crate) link: Link,
    pub(crate) registry: Registry,
    pub(crate) tasks: TestTasks,
    pub(crate) server: FakeApiServer,
    pub(crate) folder: Folder,
}

impl Bed {
    /// Starts a fake server and connects a cluster to it. Each channel is live at the return.
    pub(crate) async fn start(settings: NetSettings) -> Self {
        let server = FakeApiServer::start().await.expect("a server");
        let folder = Folder::new("bed");
        let path = folder.write("config", &kubeconfig(&[(CONTEXT, server.addr(), &server)]));
        let tasks = TestTasks::new(&Handle::current());
        let registry = registry(&tasks, &Handle::current(), &path, settings);
        let link = connect(&registry, CONTEXT);
        until_live(&link).await;
        Self { link, registry, tasks, server, folder }
    }
}

/// A folder for the files of one test. Its drop removes it.
pub(crate) struct Folder(PathBuf);

impl Folder {
    pub(crate) fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!("fresnel-core-test-{label}-{:08x}", fastrand::u32(..)));
        std::fs::create_dir(&folder).expect("a new folder");
        Self(folder)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    pub(crate) fn write(&self, file: &str, text: &str) -> PathBuf {
        let path = self.0.join(file);
        std::fs::write(&path, text).expect("a write to the test folder");
        path
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("cannot remove {}: {error}", self.0.display());
        }
    }
}

/// The certificate of a fake server as the text of a certificate file.
pub(crate) fn certificate_pem(server: &FakeApiServer) -> String {
    let certificate = STANDARD.encode(server.certificate_der());
    format!("-----BEGIN CERTIFICATE-----\n{certificate}\n-----END CERTIFICATE-----\n")
}

/// The certificate of a fake server as the value of `certificate-authority-data`.
pub(crate) fn authority(server: &FakeApiServer) -> String {
    STANDARD.encode(certificate_pem(server))
}

/// The text of a kubeconfig with one context for each server, and no user. `address` is
/// the server or a proxy before it.
pub(crate) fn kubeconfig(clusters: &[(&str, SocketAddr, &FakeApiServer)]) -> String {
    let mut text = String::from("clusters:\n");
    for (name, address, server) in clusters {
        let authority = authority(server);
        text.push_str(&format!(
            "- {{name: {name}, cluster: {{server: 'https://{address}', certificate-authority-data: {authority}}}}}\n"
        ));
    }
    text.push_str("contexts:\n");
    for (name, _, _) in clusters {
        text.push_str(&format!("- {{name: {name}, context: {{cluster: {name}}}}}\n"));
    }
    text
}

/// A read request, as `kube::core::Request` builds it: a path and no host.
pub(crate) fn get(path: &str) -> Request<Vec<u8>> {
    Request::get(path).body(Vec::new()).expect("a request")
}

/// A write request with a JSON body.
pub(crate) fn post(path: &str, body: &str) -> Request<Vec<u8>> {
    Request::post(path).body(body.as_bytes().to_vec()).expect("a request")
}
