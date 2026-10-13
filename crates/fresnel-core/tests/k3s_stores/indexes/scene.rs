//! The scene of the index tests: the objects of `scene.yaml`, and the steps that a manifest
//! cannot give.

use std::time::Duration;

use fresnel_core::cluster::Gateway;
use fresnel_core::kinds::Kind;
use fresnel_testkit::PodLine;
use kube::core::Request;
use kube::core::params::{Patch, PatchParams, PostParams};
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

use crate::bed::Cluster;

const MANIFEST: &str = include_str!("scene.yaml");
/// The namespace of the scene.
pub(crate) const NAMESPACE: &str = "scene";
/// The namespace that the scene deletes and that cannot end.
pub(crate) const ENDING: &str = "scene-ending";
/// The images of the scene come from the network at the first start of a pod.
const LIMIT: Duration = Duration::from_secs(300);
/// The cluster gives no signal for its state to a test, so the test asks again.
const STEP: Duration = Duration::from_millis(500);

/// What `kubectl get pods` shows for each pod of the scene when the scene is at rest: the
/// name or the start of the name, the column READY and the column STATUS.
pub(crate) const AT_REST: [(&str, &str, &str); 21] = [
    ("web-", "1/1", "Running"),
    ("broken-", "0/1", "ImagePullBackOff"),
    ("db-", "1/1", "Running"),
    ("agent-", "1/1", "Running"),
    ("once-", "0/1", "Completed"),
    ("fails-", "0/1", "Error"),
    ("tick-manual-", "0/1", "Completed"),
    ("loop-a-", "0/1", "Completed"),
    ("crash", "0/1", "CrashLoopBackOff"),
    ("done", "0/1", "Completed"),
    ("failed", "0/1", "Error"),
    ("init-error", "0/1", "Init:Error"),
    ("init-crash", "0/1", "Init:CrashLoopBackOff"),
    ("init-wait", "0/1", "Init:1/2"),
    ("unschedulable", "0/1", "Pending"),
    ("gated", "0/1", "SchedulingGated"),
    ("no-config", "0/1", "CreateContainerConfigError"),
    ("sidecar", "2/2", "Running"),
    ("not-ready", "0/1", "Running"),
    ("half-done", "1/2", "NotReady"),
    ("leaving", "1/1", "Terminating"),
];
/// The pods of the scene: two of `web`, two of `db`, and one for each other line.
const PODS: usize = AT_REST.len() + 2;
/// The condition `ready` of each endpoint of a Service when the scene is at rest, in the
/// sequence of the words. Behind `mixed`, one pod is not ready and one ends.
const ENDPOINTS: [(&str, &[&str]); 3] =
    [("web", &["true", "true"]), ("db", &["true", "true"]), ("mixed", &["false", "false", "true"])];
/// The namespace of the pods of k3s itself, and their count: the DNS server and the
/// storage provisioner. They are a part of the fixture, so the scene waits for them too.
const SYSTEM: &str = "kube-system";
const SYSTEM_PODS: usize = 2;

/// The collection of a kind, for all namespaces or for one.
pub(crate) fn collection(kind: Kind, namespace: Option<&str>) -> String {
    let path = kind.info().path();
    match (namespace, path.rsplit_once('/')) {
        (Some(namespace), Some((group, plural))) => format!("{group}/namespaces/{namespace}/{plural}"),
        _ => path,
    }
}

/// Creates each object of the manifest. Gives the UID of each one by its kind and name.
async fn create_manifest(api: &Gateway) -> Vec<(String, String)> {
    let documents: Vec<Value> = serde_saphyr::from_multiple(MANIFEST).expect("the manifest is YAML");
    let mut uids = Vec::new();
    for document in documents {
        let text = |pointer: &str| document.pointer(pointer).and_then(Value::as_str);
        let (kind, name) = (text("/kind").expect("a kind"), text("/metadata/name").expect("a name"));
        let known = Kind::ALL.iter().find(|known| known.info().name == kind).expect("a kind of the app");
        let request = Request::new(collection(*known, text("/metadata/namespace")));
        let body = serde_json::to_vec(&document).expect("JSON");
        let create = request.create(&PostParams::default(), body).expect("a request");
        let created: Value = api.send(create).await.unwrap_or_else(|error| panic!("create {kind} {name}: {error}"));
        let uid = created.pointer("/metadata/uid").and_then(Value::as_str).expect("a UID");
        uids.push((format!("{kind}/{name}"), uid.to_owned()));
    }
    uids
}

/// Creates the scene and waits until it is at rest. Gives the time that this took.
pub(crate) async fn create(cluster: &Cluster) -> Duration {
    let started = Instant::now();
    let api = &cluster.api.gateway;
    let uids = create_manifest(api).await;
    let uid = |object: &str| &uids.iter().find(|(known, _)| known == object).expect("an object of the manifest").1;
    let kubectl = async |arguments: &[&str]| {
        let done = cluster.k3s.kubectl(&[&["--namespace", NAMESPACE], arguments].concat()).await;
        done.unwrap_or_else(|error| panic!("kubectl {arguments:?}: {error}"))
    };

    // A second ReplicaSet for the Deployment: the first one stays, with no pod.
    let rollout = ["rollout", "status", "deployment/web", "--timeout=300s"];
    kubectl(&rollout).await;
    let annotation = json!({ "spec": { "template": { "metadata": { "annotations": { "scene/round": "2" } } } } });
    let deployments = Request::new(collection(Kind::Deployment, Some(NAMESPACE)));
    let patch = deployments.patch("web", &PatchParams::default(), &Patch::Merge(annotation)).expect("a request");
    let _: Value = api.send(patch).await.expect("the second round of the Deployment");
    kubectl(&rollout).await;

    kubectl(&["create", "job", "tick-manual", "--from=cronjob/tick"]).await;
    // Two Jobs that own each other.
    let jobs = Request::new(collection(Kind::Job, Some(NAMESPACE)));
    for (job, owner) in [("loop-a", "loop-b"), ("loop-b", "loop-a")] {
        let reference =
            json!({ "apiVersion": "batch/v1", "kind": "Job", "name": owner, "uid": uid(&format!("Job/{owner}")) });
        let owners = Patch::Merge(json!({ "metadata": { "ownerReferences": [reference] } }));
        let patch = jobs.patch(job, &PatchParams::default(), &owners).expect("a request");
        let _: Value = api.send(patch).await.unwrap_or_else(|error| panic!("the owner of {job}: {error}"));
    }
    kubectl(&["wait", "--for=condition=Ready", "pod/leaving", "--timeout=300s"]).await;
    kubectl(&["delete", "pod", "leaving", "--wait=false"]).await;
    kubectl(&["delete", "namespace", ENDING, "--wait=false"]).await;
    until_at_rest(cluster).await;
    started.elapsed()
}

/// Why the scene is not at rest. `None` when it is.
async fn unrest(cluster: &Cluster) -> Option<String> {
    let pods = pod_lines(cluster).await.1;
    let count = |namespace: &str| pods.iter().filter(|pod| pod.namespace == namespace).count();
    if (count(NAMESPACE), count(SYSTEM)) != (PODS, SYSTEM_PODS) {
        return Some(format!("{} pods of the scene and {} of k3s", count(NAMESPACE), count(SYSTEM)));
    }
    for pod in &pods {
        let Some((ready, status)) = at_rest(pod) else {
            return Some(format!("{} is not a pod of the scene", pod.name));
        };
        if (pod.ready.as_str(), pod.status.as_str()) != (ready, status) {
            return Some(format!("{} is {} {}, and not {ready} {status}", pod.name, pod.ready, pod.status));
        }
    }
    let read = async |arguments: &[&str]| cluster.k3s.kubectl(arguments).await.expect("kubectl reads the cluster");
    let progress = r#"jsonpath={.status.conditions[?(@.type=="Progressing")].reason}"#;
    let reason = read(&["get", "deployment", "broken", "--namespace", NAMESPACE, "--output", progress]).await;
    if reason != "ProgressDeadlineExceeded" {
        return Some(format!("the rollout of `broken` has the reason {reason:?}"));
    }
    let conditions = read(&["get", "namespace", ENDING, "--output", "jsonpath={.status.conditions[*].type}"]).await;
    if !conditions.contains("NamespaceFinalizersRemaining") {
        return Some(format!("the namespace `{ENDING}` has the conditions {conditions:?}"));
    }
    // The EndpointSlices follow the pods a moment later.
    for (service, at_rest) in ENDPOINTS {
        let label = format!("kubernetes.io/service-name={service}");
        let ready = "jsonpath={.items[*].endpoints[*].conditions.ready}";
        let slices = ["get", "endpointslices", "--namespace", NAMESPACE, "--selector", &label, "--output", ready];
        let mut endpoints: Vec<String> = read(&slices).await.split_whitespace().map(str::to_owned).collect();
        endpoints.sort_unstable();
        if endpoints != at_rest {
            return Some(format!("the endpoints of `{service}` are ready: {endpoints:?}"));
        }
    }
    None
}

/// What kubectl shows of each pod of the cluster now: its text, and the lines of it.
pub(crate) async fn pod_lines(cluster: &Cluster) -> (String, Vec<PodLine>) {
    let text = cluster.k3s.pod_lines().await.expect("kubectl lists the pods");
    let lines = PodLine::parse_all(&text).expect("lines of pods");
    (text, lines)
}

/// The columns READY and STATUS of a pod when the scene is at rest. `None` for a pod that
/// the scene does not know.
pub(crate) fn at_rest(pod: &PodLine) -> Option<(&'static str, &'static str)> {
    let fits = |known: &str| if known.ends_with('-') { pod.name.starts_with(known) } else { pod.name == known };
    match pod.namespace.as_str() {
        SYSTEM => Some(("1/1", "Running")),
        NAMESPACE => AT_REST.iter().find(|(known, _, _)| fits(known)).map(|(_, ready, status)| (*ready, *status)),
        _ => None,
    }
}

async fn until_at_rest(cluster: &Cluster) {
    let end = Instant::now() + LIMIT;
    while let Some(unrest) = unrest(cluster).await {
        assert!(Instant::now() < end, "the scene is not at rest after {LIMIT:?}: {unrest}");
        sleep(STEP).await;
    }
}
