//! A user of a test cluster with a client certificate, which the cluster signs through a
//! CertificateSigningRequest. The roles of the user are a part of each test.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fresnel_core::cluster::Gateway;
use fresnel_testkit::K3s;
use kube::core::Request;
use kube::core::params::{GetParams, PostParams};
use rcgen::{CertificateParams, DnType, KeyPair};
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

const CSR_PATH: &str = "/apis/certificates.k8s.io/v1/certificatesigningrequests";
/// The longest wait for the signer of the cluster.
const SIGN_LIMIT: Duration = Duration::from_secs(60);
const STEP: Duration = Duration::from_millis(200);

/// Makes the user `name` with a new key and a certificate of the cluster. Gives the
/// kubeconfig of the user: the one of the cluster admin with the certificate and the key of
/// the user. The file is in the folder of the cluster, which its drop removes.
pub(crate) async fn user(api: &Gateway, k3s: &K3s, name: &str, labels: &Value) -> PathBuf {
    let key = KeyPair::generate().expect("a key");
    let mut params = CertificateParams::new(Vec::new()).expect("the parameters of a certificate");
    params.distinguished_name.push(DnType::CommonName, name);
    let request = pem("CERTIFICATE REQUEST", params.serialize_request(&key).expect("a request").der());
    let csr = json!({
        "apiVersion": "certificates.k8s.io/v1", "kind": "CertificateSigningRequest",
        "metadata": { "name": name, "labels": labels },
        "spec": {
            "request": STANDARD.encode(request), "signerName": "kubernetes.io/kube-apiserver-client",
            "expirationSeconds": 86_400, "usages": ["client auth"],
        },
    });
    create(api, CSR_PATH, csr).await;
    k3s.kubectl(&["certificate", "approve", name]).await.expect("the approval");
    let certificate = signed(api, name).await;
    write_kubeconfig(k3s, name, &certificate, &pem("PRIVATE KEY", &key.serialize_der()))
}

/// Creates an object in a collection as the cluster admin.
pub(crate) async fn create(api: &Gateway, collection: &str, body: Value) {
    let request = Request::new(collection).create(&PostParams::default(), serde_json::to_vec(&body).expect("JSON"));
    let _: Value =
        api.send(request.expect("a request")).await.unwrap_or_else(|error| panic!("create in {collection}: {error}"));
}

/// The PEM text of DER data: the form of a kubeconfig and of the API for a request.
fn pem(label: &str, der: &[u8]) -> String {
    let base64 = STANDARD.encode(der);
    let lines: Vec<&str> =
        base64.as_bytes().chunks(64).map(|line| std::str::from_utf8(line).expect("base64")).collect();
    format!("-----BEGIN {label}-----\n{}\n-----END {label}-----\n", lines.join("\n"))
}

/// The certificate of the user, after the signer of the cluster issued it.
async fn signed(api: &Gateway, name: &str) -> Vec<u8> {
    let end = Instant::now() + SIGN_LIMIT;
    loop {
        let request = Request::new(CSR_PATH).get(name, &GetParams::default()).expect("a request");
        let csr: Value = api.get(request).await.expect("the CertificateSigningRequest");
        if let Some(certificate) = csr.pointer("/status/certificate").and_then(Value::as_str) {
            return STANDARD.decode(certificate).expect("base64");
        }
        assert!(Instant::now() < end, "the cluster signs the certificate in time: {csr}");
        sleep(STEP).await;
    }
}

fn write_kubeconfig(k3s: &K3s, name: &str, certificate: &[u8], key: &str) -> PathBuf {
    let admin = k3s.kubeconfig();
    let text = std::fs::read_to_string(&admin).expect("the kubeconfig of the cluster");
    let mut config: Value = serde_saphyr::from_str(&text).expect("YAML");
    config["users"][0]["user"] =
        json!({ "client-certificate-data": STANDARD.encode(certificate), "client-key-data": STANDARD.encode(key) });
    let path = admin.with_file_name(format!("kubeconfig-{name}.yaml"));
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).expect("a new file");
    file.write_all(serde_saphyr::to_string(&config).expect("YAML").as_bytes()).expect("a write");
    path
}
