//! Delete with each propagation policy, force delete and evict: the app on an object leaves
//! the state that kubectl leaves for its twin.

use fresnel_core::kinds::Kind;
use fresnel_core::ops::{JobState, Operation, Order, Propagation, Target};
use serde_json::Value as Json;

use crate::bed::{APP, Cluster, KUBECTL};

async fn gone(b: &Cluster, namespace: &str, kind: Kind, name: &str) -> bool {
    b.object(namespace, kind, name).await.is_none()
}

async fn no_pods(b: &Cluster, namespace: &str, selector: &str) -> bool {
    b.names(namespace, Kind::Pod, selector).await.is_empty()
}

/// A Job goes with its pods, as with `kubectl delete`: the default policy of the server for
/// a Job would keep them. A ReplicaSet with the policy `Orphan` leaves its pods with no owner.
pub(crate) async fn delete(b: &Cluster) {
    let report = b.run(Kind::Job, "doomed", Operation::Delete(Propagation::Background)).await;
    assert_eq!(report.state, JobState::Done, "{report:#?}");
    b.kubectl(&["delete", "jobs.batch", "doomed", "--namespace", KUBECTL, "--wait=false"]).await;
    for namespace in [APP, KUBECTL] {
        b.until("the Job is gone", || gone(b, namespace, Kind::Job, "doomed")).await;
        b.until("the pods of the Job are gone", || no_pods(b, namespace, "app=doomed")).await;
    }

    let report = b.run(Kind::ReplicaSet, "orphans", Operation::Delete(Propagation::Orphan)).await;
    assert_eq!(report.state, JobState::Done, "{report:#?}");
    b.kubectl(&["delete", "replicasets.apps", "orphans", "--cascade=orphan", "--namespace", KUBECTL]).await;
    for namespace in [APP, KUBECTL] {
        b.until("the ReplicaSet is gone", || gone(b, namespace, Kind::ReplicaSet, "orphans")).await;
        b.until("the pods stay with no owner", || orphans(b, namespace)).await;
    }
}

/// True when the ReplicaSet `orphans` left pods, and none of them has an owner.
async fn orphans(b: &Cluster, namespace: &str) -> bool {
    let pods = b.names(namespace, Kind::Pod, "app=orphans").await;
    let mut free = !pods.is_empty();
    for pod in &pods {
        let object = b.object(namespace, Kind::Pod, pod).await.unwrap_or_default();
        free &= object.pointer("/metadata/ownerReferences").and_then(Json::as_array).is_none_or(Vec::is_empty);
    }
    free
}

/// `kubectl delete --force --grace-period=0`: the pod object goes at once.
pub(crate) async fn force_delete(b: &Cluster) {
    let report = b.run(Kind::Pod, "lone", Operation::ForceDelete).await;
    assert_eq!(report.state, JobState::Done, "{report:#?}");
    assert!(gone(b, APP, Kind::Pod, "lone").await, "the server removed the pod at once");
    b.kubectl(&["delete", "pod", "lone", "--force", "--grace-period=0", "--namespace", KUBECTL]).await;
    assert!(gone(b, KUBECTL, Kind::Pod, "lone").await);
}

/// The UID of the object that the user saw is a precondition: an other object of the name
/// stays, and the server refuses with HTTP 409.
pub(crate) async fn precondition(b: &Cluster) {
    let target = Target { uid: "an-older-pod".into(), ..b.target(Kind::Pod, "evictee").await };
    for operation in [Operation::Delete(Propagation::Background), Operation::ForceDelete, Operation::Evict] {
        let order = Order::new(target.clone(), operation).expect("an order");
        let report = b.run_order(&order, fresnel_core::ops::Cancel::pair().1).await;
        let JobState::Failed(error) = &report.state else { panic!("a refusal: {report:#?}") };
        assert_eq!(error.status(), Some(409), "{error}");
    }
    assert!(!gone(b, APP, Kind::Pod, "evictee").await, "the pod stays");
}

/// An eviction removes a pod as `kubectl drain` does, and a PodDisruptionBudget blocks it
/// with HTTP 429 and its message.
pub(crate) async fn evict(b: &Cluster) {
    let report = b.run(Kind::Pod, "evictee", Operation::Evict).await;
    assert_eq!(report.state, JobState::Done, "{report:#?}");
    b.until("the evicted pod is gone", || gone(b, APP, Kind::Pod, "evictee")).await;
    // The pod of the app is gone, so the selector of the drain takes the twin only.
    let node = b.kubectl(&["get", "nodes", "--output", "jsonpath={.items[0].metadata.name}"]).await;
    let drain = ["drain", node.trim(), "--pod-selector", "app=evictee", "--force", "--ignore-daemonsets"];
    b.kubectl(&[&drain[..], &["--delete-emptydir-data", "--timeout=120s"]].concat()).await;
    b.kubectl(&["uncordon", node.trim()]).await;
    assert!(gone(b, KUBECTL, Kind::Pod, "evictee").await, "kubectl evicted the twin");

    // A pending pod skips the budget, so the test waits until the budget counts the pod.
    let healthy = || async {
        let budget = b.object(APP, Kind::PodDisruptionBudget, "guarded").await.unwrap_or_default();
        budget.pointer("/status/currentHealthy") == Some(&Json::from(1))
    };
    b.until("the budget counts its pod", healthy).await;
    let guarded = b.names(APP, Kind::Pod, "app=guarded").await;
    let pod = guarded.first().expect("the pod of the Deployment with a budget");
    let report = b.run(Kind::Pod, pod, Operation::Evict).await;
    let JobState::Failed(error) = &report.state else { panic!("a refusal: {report:#?}") };
    assert_eq!(error.status(), Some(429));
    let message = "Cannot evict pod as it would violate the pod's disruption budget.";
    assert!(error.to_string().starts_with(message), "{error}");
    assert!(!gone(b, APP, Kind::Pod, pod).await, "the budget keeps the pod");
}
