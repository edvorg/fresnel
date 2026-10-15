//! The samples of each series of a cluster, and the requests on its node as kubectl
//! describes them.

use fresnel_core::kinds::Kind;
use fresnel_core::metrics::{Cover, Metrics, SeriesKey};
use fresnel_core::store::ObjectKey;
use serde_json::json;

use crate::bed::{Cluster, NODE, until};

/// A Deployment whose pod uses CPU in a loop, with a limit of 300m.
fn burner() -> serde_json::Value {
    let container = json!({"name": "loop", "image": "rancher/mirrored-library-busybox:1.36.1",
        "command": ["sh", "-c", "while :; do :; done"],
        "resources": {"requests": {"cpu": "100m", "memory": "16Mi"}, "limits": {"cpu": "300m", "memory": "32Mi"}}});
    json!({"apiVersion": "apps/v1", "kind": "Deployment", "metadata": {"name": "burn"},
        "spec": {"replicas": 1, "selector": {"matchLabels": {"app": "burn"}}, "template": {
            "metadata": {"labels": {"app": "burn"}},
            "spec": {"terminationGracePeriodSeconds": 0, "containers": [container]}}}})
}

/// The name of the one object of a kind with the label `app=burn`.
async fn burner_name(cluster: &Cluster, resource: &str) -> String {
    let text = cluster.kubectl(&["get", resource, "--selector", "app=burn", "--output", "name"]).await;
    let names: Vec<&str> = text.lines().filter_map(|line| line.split_once('/')).map(|(_, name)| name).collect();
    assert_eq!(names.len(), 1, "one {resource} of the burner: {names:?}");
    names.concat()
}

/// The pod of the burner runs, and each of its series has a value of the last answer: its
/// container, the pod, its ReplicaSet and Deployment, the namespace, the node and the cluster.
pub(crate) async fn of_each_series_arrive(a: &Cluster) {
    a.create(Kind::Deployment, &burner()).await;
    let burn = SeriesKey::Workload(Kind::Deployment, ObjectKey::new("default", "burn"));
    let ready = |metrics: &Metrics| {
        let latest = metrics.latest(&a.id, &burn).filter(|latest| latest.current);
        latest.is_some_and(|latest| latest.sample.cover == Cover::ONE && latest.sample.usage.cpu > 50_000_000)
    };
    let metrics = until(&a.metrics, "the burner has a value", ready).await;
    let (pod, set) = (burner_name(a, "pods").await, burner_name(a, "replicasets").await);
    let pod = ObjectKey::new("default", &pod);
    let keys = [
        SeriesKey::Container { pod: pod.clone(), name: "loop".into() },
        SeriesKey::Pod(pod.clone()),
        SeriesKey::Workload(Kind::ReplicaSet, ObjectKey::new("default", &set)),
        burn,
        SeriesKey::Namespace("default".into()),
        SeriesKey::Node(NODE.into()),
        SeriesKey::Cluster,
    ];
    for key in &keys {
        let latest = metrics.latest(&a.id, key).unwrap_or_else(|| panic!("a value of {key:?}"));
        assert!(latest.current, "{key:?} is current");
        assert!(latest.sample.usage.memory > 0, "{key:?} has memory");
        // The time of the server is the time of the sample, and it is not later than the
        // answer: the poll does not stamp its own time.
        let answered = metrics.pods.answered.expect("an answer");
        assert!(latest.sample.time() <= answered + std::time::Duration::from_secs(5), "{key:?}");
    }
    assert_eq!(metrics.latest(&a.id, &SeriesKey::Cluster).map(|latest| latest.sample.cover), Some(Cover::ONE));
    let names: Vec<&str> = metrics.containers(&a.id, &pod).map(|(name, _)| name).collect();
    assert_eq!(names, ["loop"]);
    the_requests_of_the_node_are_the_ones_of_kubectl(a, &metrics).await;
}

/// `kubectl describe node` counts the requests of the pods that are not done, as the app.
async fn the_requests_of_the_node_are_the_ones_of_kubectl(a: &Cluster, metrics: &Metrics) {
    let text = a.kubectl(&["describe", "node", NODE]).await;
    let allocated = text.split_once("Allocated resources:").map(|(_, rest)| rest).expect("the allocated resources");
    let cpu = allocated.lines().find_map(|line| line.trim().strip_prefix("cpu")).expect("the line of the CPU");
    let millicores = cpu.split_whitespace().next().and_then(|value| value.strip_suffix('m')).expect("millicores");
    let pods =
        text.split_once("Non-terminated Pods:").and_then(|(_, rest)| rest.split_once(" in total")).expect("pods");
    let pods: u32 = pods.0.trim().trim_start_matches('(').parse().expect("a count of pods");
    let load = metrics.node_load(&a.id, NODE).expect("the load of the node");
    assert_eq!((load.requests.cpu / 1_000_000).to_string(), millicores, "the CPU requests of kubectl");
    assert_eq!(load.pods, pods, "the pods of kubectl");
}
