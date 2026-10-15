//! One kubeconfig for the core of a test with many clusters: k3s writes a file for each
//! cluster, and each file names its context `default`.

use std::path::PathBuf;

use serde_json::{Value as Json, json};

/// One kubeconfig with a context for each file: its cluster and its user, under the name
/// of the context. The first context is the current one.
pub(crate) fn kubeconfig(contexts: &[(&str, PathBuf)]) -> String {
    let (mut clusters, mut users, mut named) = (Vec::new(), Vec::new(), Vec::new());
    for (name, file) in contexts {
        let text = std::fs::read_to_string(file).expect("the kubeconfig of a cluster");
        let config: Json = serde_saphyr::from_str(&text).expect("YAML");
        clusters.push(json!({"name": name, "cluster": config["clusters"][0]["cluster"]}));
        users.push(json!({"name": name, "user": config["users"][0]["user"]}));
        named.push(json!({"name": name, "context": {"cluster": name, "user": name}}));
    }
    let current = contexts.first().map(|(name, _)| *name).unwrap_or_default();
    let config = json!({"apiVersion": "v1", "kind": "Config", "current-context": current,
        "clusters": clusters, "users": users, "contexts": named});
    serde_saphyr::to_string(&config).expect("YAML")
}
