//! A user of the test cluster who can read each kind of the app but the ConfigMaps. The
//! cluster signs a client certificate for the user (`common/user.rs`).

use std::path::PathBuf;

use fresnel_core::cluster::{Link, Registry};
use fresnel_core::kinds::Kind;
use fresnel_core::net::NetSettings;
use fresnel_core::store::StoreSettings;
use serde_json::{Value, json};
use tokio::runtime::Handle;

use super::scene::LABEL;
use crate::bed::Cluster;
use crate::common::{connect, registry};
use crate::store_bed::Stores;
use crate::user::{create, user};

/// The name of the user, of its CertificateSigningRequest and of its ClusterRole.
const NAME: &str = "fresnel-reader";

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
        let labels = json!({ LABEL.0: LABEL.1 });
        let kubeconfig = user(api, &cluster.k3s, NAME, &labels).await;
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
        Self { kubeconfig }
    }

    /// Connects the user to the cluster and starts each store.
    pub(crate) fn app(&self, cluster: &Cluster) -> ReaderApp {
        let registry = registry(&cluster.tasks, &Handle::current(), &self.kubeconfig, NetSettings::default());
        let link = connect(&registry, "default");
        let stores = Stores::start(&cluster.tasks, &link, StoreSettings::default());
        ReaderApp { stores, _link: link, _registry: registry }
    }
}
