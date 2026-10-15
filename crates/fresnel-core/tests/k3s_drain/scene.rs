//! The pods of the test. Each node of a drain has the same set: the namespace `app` on the
//! node `drained`, the namespace `kubectl` on its twin, and a DaemonSet of the namespace
//! `shared` on each node. The pods use the pause image of k3s, which each node has.
//!
//! - `web`: a Deployment, which the drain evicts. `scratch`: a Deployment with `emptyDir`.
//! - `guarded`: a Deployment with a PodDisruptionBudget that allows no disruption.
//! - `lone`: a pod with no controller. `ended`: a pod with no controller that failed.

use kube::core::Request;
use kube::core::params::PostParams;
use serde_json::{Value as Json, json};

use crate::bed::{Cluster, TAINT};

const IMAGE: &str = "rancher/mirrored-pause:3.6";
/// The namespace of the DaemonSet.
pub(crate) const SHARED: &str = "shared";

/// The pod template of `name` on a node.
fn template(name: &str, node: &str) -> Json {
    json!({"metadata": {"labels": {"app": name}}, "spec": {
        "nodeSelector": {"kubernetes.io/hostname": node},
        "tolerations": [{"key": TAINT, "operator": "Exists"}],
        "terminationGracePeriodSeconds": 0,
        "containers": [{"name": "c", "image": IMAGE, "imagePullPolicy": "IfNotPresent"}]}})
}

fn deployment(name: &str, template: Json) -> (&'static str, Json) {
    let spec = json!({"replicas": 1, "selector": {"matchLabels": {"app": name}}, "template": template});
    ("/apis/apps/v1/namespaces/{}/deployments", json!({"metadata": {"name": name}, "spec": spec}))
}

fn pod(name: &str, mut template: Json) -> (&'static str, Json) {
    template["metadata"]["name"] = name.into();
    ("/api/v1/namespaces/{}/pods", template)
}

/// The objects of a namespace whose pods run on `node`, as (collection, object).
fn objects(node: &str) -> Vec<(&'static str, Json)> {
    let mut scratch = template("scratch", node);
    scratch["spec"]["volumes"] = json!([{"name": "scratch", "emptyDir": {}}]);
    let mut ended = template("ended", node);
    ended["spec"]["restartPolicy"] = "Never".into();
    ended["spec"]["activeDeadlineSeconds"] = 1.into();
    let budget = json!({"metadata": {"name": "guarded"},
        "spec": {"minAvailable": 1, "selector": {"matchLabels": {"app": "guarded"}}}});
    vec![
        deployment("web", template("web", node)),
        deployment("guarded", template("guarded", node)),
        deployment("scratch", scratch),
        ("/apis/policy/v1/namespaces/{}/poddisruptionbudgets", budget),
        pod("lone", template("lone", node)),
        pod("ended", ended),
    ]
}

/// Creates the namespaces with their pods on their nodes, and with `daemon_set` the
/// DaemonSet on each node, and waits until each pod is in its state.
pub(crate) async fn create(cluster: &Cluster, namespaces: &[(&str, &str)], daemon_set: bool) {
    let created = |namespace: &str| (Request::new("/api/v1/namespaces"), json!({"metadata": {"name": namespace}}));
    for (namespace, node) in namespaces {
        let (request, object) = created(namespace);
        send(cluster, request.create(&PostParams::default(), body(&object))).await;
        for (collection, object) in objects(node) {
            let request = Request::new(collection.replace("{}", namespace));
            send(cluster, request.create(&PostParams::default(), body(&object))).await;
        }
    }
    if daemon_set {
        let (request, object) = created(SHARED);
        send(cluster, request.create(&PostParams::default(), body(&object))).await;
        let mut template = template("agent", "");
        template["spec"]["tolerations"] = json!([{"operator": "Exists"}]);
        template["spec"].as_object_mut().expect("a spec").remove("nodeSelector");
        let spec = json!({"selector": {"matchLabels": {"app": "agent"}}, "template": template});
        let set = json!({"metadata": {"name": "agent"}, "spec": spec});
        let request = Request::new(format!("/apis/apps/v1/namespaces/{SHARED}/daemonsets"));
        send(cluster, request.create(&PostParams::default(), body(&set))).await;
        cluster.kubectl(&["rollout", "status", "daemonset/agent", "-n", SHARED, "--timeout=180s"]).await;
    }
    for (namespace, _) in namespaces {
        for name in ["web", "guarded", "scratch"] {
            let object = format!("deployment/{name}");
            cluster.kubectl(&["rollout", "status", &object, "-n", namespace, "--timeout=180s"]).await;
        }
        let wait = |condition: &'static str, object: &'static str| {
            ["wait", condition, object, "-n", namespace, "--timeout=180s"]
        };
        cluster.kubectl(&wait("--for=condition=Ready", "pod/lone")).await;
        cluster.kubectl(&wait("--for=jsonpath={.status.phase}=Failed", "pod/ended")).await;
        // A pod that is not ready skips the budget, so the budget must count the pod first.
        cluster.kubectl(&wait("--for=jsonpath={.status.currentHealthy}=1", "pdb/guarded")).await;
    }
}

fn body(object: &Json) -> Vec<u8> {
    serde_json::to_vec(object).expect("JSON")
}

async fn send(cluster: &Cluster, request: Result<http::Request<Vec<u8>>, kube::core::request::Error>) {
    let request = request.expect("a request");
    let path = request.uri().to_string();
    let answer: Result<Json, _> = cluster.admin.gateway.send(request).await;
    answer.unwrap_or_else(|error| panic!("{path}: {error}"));
}
