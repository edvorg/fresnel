//! The k3s helper against Docker: the start, the API address, a restart and the cleanup.
//!
//! The steps are in one test, because the start of a cluster is the slow part.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "common/client.rs"]
mod client;

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fresnel_testkit::K3s;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpStream;

use client::{Trust, in_time, text};

/// The stream limit of a kube-apiserver. The supervisor port of k3s gives 250.
const API_SERVER_STREAMS: usize = 100;

fn docker(arguments: &[&str]) -> String {
    let output = Command::new("docker").args(arguments).output().expect("docker starts");
    assert!(output.status.success(), "docker {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).expect("text").trim().to_owned()
}

/// One value of the kubeconfig, decoded from its base64 form.
fn decoded(kubeconfig: &str, key: &str) -> Vec<u8> {
    let line = kubeconfig.lines().find_map(|line| line.trim().strip_prefix(key)).expect("the key is in the file");
    STANDARD.decode(line.trim()).expect("base64")
}

/// The identity of the cluster admin and the trusted certificate, from the kubeconfig file.
fn admin(kubeconfig: &Path) -> Trust {
    let text = std::fs::read_to_string(kubeconfig).expect("the kubeconfig file");
    let certificates = |key| {
        let pem = decoded(&text, key);
        CertificateDer::pem_slice_iter(&pem).collect::<Result<Vec<_>, _>>().expect("certificates")
    };
    let key = PrivateKeyDer::from_pem_slice(&decoded(&text, "client-key-data:")).expect("a key");
    Trust {
        roots: certificates("certificate-authority-data:"),
        identity: Some((certificates("client-certificate-data:"), key)),
    }
}

fn server_line(kubeconfig: &Path) -> String {
    let text = std::fs::read_to_string(kubeconfig).expect("the kubeconfig file");
    text.lines().find(|line| line.contains("server:")).expect("a server line").trim().to_owned()
}

/// Asks the API server through the address of the helper. Gives the status, the body and
/// the stream limit of the server.
async fn ask(cluster: &K3s, path: &str) -> (u16, String, usize) {
    let mut connection = admin(&cluster.kubeconfig()).http2(cluster.api_addr()).await;
    let response = connection.get(path).await;
    let limit = connection.stream_limit();
    (response.status().as_u16(), text(response).await, limit)
}

fn container_address(cluster: &K3s) -> String {
    docker(&["inspect", "--format", "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}", cluster.name()])
}

#[tokio::test]
#[ignore = "needs k3s in Docker"]
async fn the_helper_gives_the_kube_apiserver_and_keeps_its_address_and_removes_the_container() {
    let cluster = K3s::start("kit").await.expect("k3s starts");
    let (name, api, kubeconfig) = (cluster.name().to_owned(), cluster.api_addr(), cluster.kubeconfig());
    assert!(name.starts_with("fresnel-test-kit-"), "{name}");
    assert_eq!(docker(&["port", &name]), format!("6444/tcp -> {api}"), "only the kube-apiserver port is published");
    assert_eq!(server_line(&kubeconfig), format!("server: https://{api}"));
    let mode = std::fs::metadata(&kubeconfig).expect("metadata").permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "the file holds the key of the admin");

    let (status, version, limit) = ask(&cluster, "/version").await;
    assert_eq!(status, 200, "the admin of the kubeconfig can read: {version}");
    assert!(version.contains("v1.33.5+k3s1"), "{version}");
    assert_eq!(limit, API_SERVER_STREAMS, "the address is the kube-apiserver and not the supervisor");

    cluster.kubectl(&["create", "namespace", "kit-probe"]).await.expect("kubectl works");
    assert_eq!(ask(&cluster, "/api/v1/namespaces/kit-probe").await.0, 200);
    assert_eq!(ask(&cluster, "/api/v1/namespaces/kit-absent").await.0, 404);

    let address = container_address(&cluster);
    assert!(address.starts_with("10.199.") && address.ends_with(".2"), "{address}");
    cluster.restart().await.expect("the restart");
    assert_eq!(container_address(&cluster), address, "the address is fixed");
    assert_eq!(ask(&cluster, "/api/v1/namespaces/kit-probe").await.0, 200, "the same address and the same data");

    cluster.stop().await.expect("the stop");
    assert!(in_time(TcpStream::connect(api)).await.is_err(), "a stopped cluster refuses the connect");
    cluster.resume().await.expect("the start");
    assert_eq!((container_address(&cluster), ask(&cluster, "/version").await.0), (address, 200));

    let proxy: SocketAddr = "127.0.0.1:50123".parse().expect("an address");
    let through = cluster.kubeconfig_through(proxy).expect("a second kubeconfig");
    assert_eq!(server_line(&through), "server: https://127.0.0.1:50123");
    assert_eq!(server_line(&kubeconfig), format!("server: https://{api}"), "the first file does not change");

    let folder = kubeconfig.parent().expect("a folder").to_owned();
    drop(cluster);
    let filter = format!("name={name}");
    assert_eq!(docker(&["ps", "--all", "--quiet", "--filter", &filter]), "", "the container is gone");
    assert_eq!(docker(&["network", "ls", "--quiet", "--filter", &filter]), "", "the network is gone");
    assert!(!folder.exists(), "the kubeconfig files are gone");
}
