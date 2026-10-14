//! The objects of the test. Each cluster has the same objects in the namespace of the app
//! and in the namespace of kubectl, so an object and its twin differ only by the change of
//! the operation. The pods use the pause image of k3s, which each node has.

use fresnel_core::kinds::Kind;
use kube::core::Request;
use kube::core::params::{Patch, PatchParams, PostParams};
use serde_json::{Value as Json, json};

use crate::bed::{APP, Cluster, KUBECTL};

const IMAGE: &str = "rancher/mirrored-pause:3.6";

/// The pod template of an object with the label `app=<name>` and the value of `X`.
pub(crate) fn template(name: &str, x: &str) -> Json {
    json!({"metadata": {"labels": {"app": name}}, "spec": {"terminationGracePeriodSeconds": 0,
        "containers": [{"name": "c", "image": IMAGE, "imagePullPolicy": "IfNotPresent", "env": [{"name": "X", "value": x}]}]}})
}

fn workload(kind: &str, name: &str, spec: Json) -> (Kind, Json) {
    let group = Kind::named(kind).expect("a kind of the app");
    let version = if group.info().group.is_empty() { "v1".to_owned() } else { format!("{}/v1", group.info().group) };
    (
        group,
        json!({"apiVersion": version, "kind": kind, "metadata": {"name": name, "annotations": {"team": "shop"}}, "spec": spec}),
    )
}

fn selected(name: &str, mut spec: Json) -> Json {
    spec["selector"] = json!({"matchLabels": {"app": name}});
    spec["template"] = template(name, "1");
    spec
}

/// Each object of a namespace, in the sequence of their create.
fn objects() -> Vec<(Kind, Json)> {
    let mut stuck = template("stuck", "1");
    stuck["spec"]["containers"][0]["image"] = json!("rancher/not-there:0");
    let job = |name: &str| {
        json!({"template": {"metadata": {"labels": {"app": name}}, "spec": {"restartPolicy": "Never",
        "terminationGracePeriodSeconds": 0, "containers": [{"name": "c", "image": IMAGE, "imagePullPolicy": "IfNotPresent"}]}}})
    };
    vec![
        workload("Deployment", "web", selected("web", json!({"replicas": 1}))),
        workload("Deployment", "paused", selected("paused", json!({"replicas": 1, "paused": true}))),
        workload(
            "Deployment",
            "stuck",
            json!({"replicas": 1, "selector": {"matchLabels": {"app": "stuck"}}, "template": stuck}),
        ),
        workload("Deployment", "guarded", selected("guarded", json!({"replicas": 1}))),
        workload("StatefulSet", "db", selected("db", json!({"replicas": 1, "serviceName": "db"}))),
        workload("DaemonSet", "agent", selected("agent", json!({}))),
        workload("ReplicaSet", "set", selected("set", json!({"replicas": 1}))),
        workload("ReplicaSet", "orphans", selected("orphans", json!({"replicas": 1}))),
        workload(
            "ReplicationController",
            "legacy",
            json!({"replicas": 1, "selector": {"app": "legacy"}, "template": template("legacy", "1")}),
        ),
        workload("Job", "once", job("once")),
        workload("Job", "doomed", job("doomed")),
        workload(
            "CronJob",
            "nightly",
            json!({"schedule": "0 0 1 1 *", "jobTemplate": {
            "metadata": {"labels": {"app": "nightly"}, "annotations": {"team": "shop"}}, "spec": job("nightly")}}),
        ),
        workload(
            "PodDisruptionBudget",
            "guarded",
            json!({"minAvailable": 1, "selector": {"matchLabels": {"app": "guarded"}}}),
        ),
        (
            Kind::Pod,
            json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "lone", "labels": {"app": "lone"}}, "spec": template("lone", "1")["spec"]}),
        ),
        (
            Kind::Pod,
            json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "evictee", "labels": {"app": "evictee"}}, "spec": template("evictee", "1")["spec"]}),
        ),
    ]
}

/// Creates the namespaces and the objects in a cluster, and gives the workloads with a
/// revision history their second revision.
pub(crate) async fn create(cluster: &Cluster) {
    for namespace in [APP, KUBECTL] {
        send(
            cluster,
            Request::new("/api/v1/namespaces")
                .create(&PostParams::default(), body(&json!({"metadata": {"name": namespace}}))),
        )
        .await;
        for (kind, object) in objects() {
            let request = Request::new(kind.info().path_in(namespace)).create(&PostParams::default(), body(&object));
            send(cluster, request).await;
        }
        for (kind, name) in [(Kind::Deployment, "web"), (Kind::StatefulSet, "db"), (Kind::DaemonSet, "agent")] {
            let patch = Patch::Merge(json!({"spec": {"template": template(name, "2")}}));
            let request = Request::new(kind.info().path_in(namespace)).patch(name, &PatchParams::default(), &patch);
            send(cluster, request).await;
        }
    }
    for namespace in [APP, KUBECTL] {
        for (kind, name) in [(Kind::Deployment, "web"), (Kind::StatefulSet, "db"), (Kind::DaemonSet, "agent")] {
            cluster.until("two revisions", || revisions(cluster, namespace, kind, name, 2)).await;
        }
    }
}

/// True when kubectl lists `count` revisions of the workload.
async fn revisions(cluster: &Cluster, namespace: &str, kind: Kind, name: &str, count: usize) -> bool {
    revision_numbers(cluster, namespace, kind, name).await.len() == count
}

/// The numbers of the revisions in the history of kubectl.
pub(crate) async fn revision_numbers(cluster: &Cluster, namespace: &str, kind: Kind, name: &str) -> Vec<i64> {
    let object = format!("{}/{name}", crate::bed::resource(kind));
    let text = cluster.kubectl(&["rollout", "history", &object, "--namespace", namespace]).await;
    text.lines().filter_map(|line| line.split_whitespace().next()?.parse().ok()).collect()
}

fn body(object: &Json) -> Vec<u8> {
    serde_json::to_vec(object).expect("JSON")
}

async fn send(cluster: &Cluster, request: Result<http::Request<Vec<u8>>, kube::core::request::Error>) {
    let request = request.expect("a request");
    let path = request.uri().to_string();
    let answer: Result<Json, _> = cluster.link.gateway.send(request).await;
    answer.unwrap_or_else(|error| panic!("{path}: {error}"));
}
