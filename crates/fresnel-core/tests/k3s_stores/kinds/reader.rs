//! A user of the test cluster who can read each kind of the app but the ConfigMaps. The
//! cluster signs a client certificate for the user through a CertificateSigningRequest.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fresnel_core::cluster::{Gateway, Link, Registry};
use fresnel_core::kinds::Kind;
use fresnel_core::net::NetSettings;
use fresnel_core::store::StoreSettings;
use kube::core::Request;
use kube::core::params::{GetParams, PostParams};
use rcgen::{CertificateParams, DnType, KeyPair};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::time::{Instant, sleep};

use super::STEP;
use super::scene::LABEL;
use crate::bed::{Cluster, LIMIT};
use crate::common::{connect, registry};
use crate::store_bed::Stores;

/// The name of the user, of its CertificateSigningRequest and of its ClusterRole.
const NAME: &str = "fresnel-reader";
const CSR_PATH: &str = "/apis/certificates.k8s.io/v1/certificatesigningrequests";

/// The kubeconfig of the user.
pub(crate) struct Reader {
    kubeconfig: PathBuf,
}

/// The stores of the cluster as the user reads them. The stores drop first.
pub(crate) struct ReaderApp {
    pub(crate) stores: Stores,
    _link: Link,
    _registry: Registry,
}

impl Reader {
    /// Makes the user: its key, its certificate, its role and the binding of the role.
    pub(crate) async fn create(cluster: &Cluster) -> Self {
        let api = &cluster.api.gateway;
        let key = KeyPair::generate().expect("a key");
        let mut params = CertificateParams::new(Vec::new()).expect("the parameters of a certificate");
        params.distinguished_name.push(DnType::CommonName, NAME);
        let request = pem("CERTIFICATE REQUEST", params.serialize_request(&key).expect("a request").der());
        let labels = json!({ LABEL.0: LABEL.1 });
        create(
            api,
            CSR_PATH,
            json!({
                "apiVersion": "certificates.k8s.io/v1", "kind": "CertificateSigningRequest",
                "metadata": { "name": NAME, "labels": labels },
                "spec": {
                    "request": STANDARD.encode(request), "signerName": "kubernetes.io/kube-apiserver-client",
                    "expirationSeconds": 86_400, "usages": ["client auth"],
                },
            }),
        )
        .await;
        cluster.k3s.kubectl(&["certificate", "approve", NAME]).await.expect("the approval");
        let certificate = signed(api).await;

        let rules: Vec<Value> = Kind::ALL
            .iter()
            .filter(|kind| **kind != Kind::ConfigMap)
            .map(|kind| json!({ "apiGroups": [kind.info().group], "resources": [kind.info().plural], "verbs": ["get", "list", "watch"] }))
            .collect();
        let rbac = "/apis/rbac.authorization.k8s.io/v1";
        let role = json!({ "metadata": { "name": NAME, "labels": labels }, "rules": rules });
        create(api, &format!("{rbac}/clusterroles"), role).await;
        let binding = json!({
            "metadata": { "name": NAME, "labels": labels },
            "roleRef": { "apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": NAME },
            "subjects": [{ "apiGroup": "rbac.authorization.k8s.io", "kind": "User", "name": NAME }],
        });
        create(api, &format!("{rbac}/clusterrolebindings"), binding).await;
        Self { kubeconfig: write_kubeconfig(cluster, &certificate, &pem("PRIVATE KEY", &key.serialize_der())) }
    }

    /// Connects the user to the cluster and starts each store.
    pub(crate) fn app(&self, cluster: &Cluster) -> ReaderApp {
        let registry = registry(&cluster.tasks, &Handle::current(), &self.kubeconfig, NetSettings::default());
        let link = connect(&registry, "default");
        let stores = Stores::start(&cluster.tasks, &link, StoreSettings::default());
        ReaderApp { stores, _link: link, _registry: registry }
    }
}

/// The PEM text of DER data: the form of a kubeconfig and of the API for a request.
fn pem(label: &str, der: &[u8]) -> String {
    let base64 = STANDARD.encode(der);
    let lines: Vec<&str> =
        base64.as_bytes().chunks(64).map(|line| std::str::from_utf8(line).expect("base64")).collect();
    format!("-----BEGIN {label}-----\n{}\n-----END {label}-----\n", lines.join("\n"))
}

async fn create(api: &Gateway, collection: &str, body: Value) {
    let request = Request::new(collection).create(&PostParams::default(), serde_json::to_vec(&body).expect("JSON"));
    let _: Value =
        api.send(request.expect("a request")).await.unwrap_or_else(|error| panic!("create in {collection}: {error}"));
}

/// The certificate of the user, after the signer of the cluster issued it.
async fn signed(api: &Gateway) -> Vec<u8> {
    let end = Instant::now() + LIMIT;
    loop {
        let request = Request::new(CSR_PATH).get(NAME, &GetParams::default()).expect("a request");
        let csr: Value = api.get(request).await.expect("the CertificateSigningRequest");
        if let Some(certificate) = csr.pointer("/status/certificate").and_then(Value::as_str) {
            return STANDARD.decode(certificate).expect("base64");
        }
        assert!(Instant::now() < end, "the cluster signs the certificate in time: {csr}");
        sleep(STEP).await;
    }
}

/// The kubeconfig of the cluster admin, with the certificate and the key of the user. The
/// file is in the folder of the cluster, which its drop removes.
fn write_kubeconfig(cluster: &Cluster, certificate: &[u8], key: &str) -> PathBuf {
    let admin = cluster.k3s.kubeconfig();
    let text = std::fs::read_to_string(&admin).expect("the kubeconfig of the cluster");
    let mut config: Value = serde_saphyr::from_str(&text).expect("YAML");
    config["users"][0]["user"] =
        json!({ "client-certificate-data": STANDARD.encode(certificate), "client-key-data": STANDARD.encode(key) });
    let path = admin.with_file_name(format!("kubeconfig-{NAME}.yaml"));
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).expect("a new file");
    file.write_all(serde_saphyr::to_string(&config).expect("YAML").as_bytes()).expect("a write");
    path
}
