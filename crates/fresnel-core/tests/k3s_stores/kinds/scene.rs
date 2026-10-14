//! The scene of the kinds test: the objects of `scene.yaml`, and the wait until each one
//! has the state that its columns show.

use std::time::Duration;

use tokio::time::Instant;

use crate::api::create_each;
use crate::bed::Cluster;

const MANIFEST: &str = include_str!("scene.yaml");
/// The namespace of the scene.
pub(crate) const NAMESPACE: &str = "kinds";
/// The label of each object of the scene.
pub(crate) const LABEL: (&str, &str) = ("fresnel-test", "kinds");
/// The images of the scene are in k3s, so the scene is at rest in seconds.
const LIMIT: &str = "--timeout=180s";

/// What `kubectl wait` waits for in the namespace of the scene: the state that each
/// column of the tables reads. The server gives no other signal for these states.
const AT_REST: [&[&str]; 9] = [
    &["rollout", "status", "deployment/web"],
    &["rollout", "status", "statefulset/db"],
    &["rollout", "status", "daemonset/agent"],
    &["wait", "--for=jsonpath={.status.readyReplicas}=1", "replicationcontroller/legacy"],
    &["wait", "--for=jsonpath={.status.ready}=2", "job/pair"],
    &["wait", "--for=jsonpath={.status.currentHealthy}=1", "poddisruptionbudget/web", "poddisruptionbudget/db"],
    &["wait", "--for=jsonpath={.status.used.pods}=6", "resourcequota/quota"],
    &["wait", "--for=condition=ScalingActive=False", "horizontalpodautoscaler/web"],
    &["wait", "--for=condition=Available=False", "apiservice/v1alpha1.fresnel.test"],
];

/// Creates the scene and waits until it is at rest. Gives the time that this took.
pub(crate) async fn create(cluster: &Cluster) -> Duration {
    let started = Instant::now();
    create_each(&cluster.api.gateway, MANIFEST).await;
    for step in AT_REST {
        let arguments = [&["--namespace", NAMESPACE], step, &[LIMIT]].concat();
        cluster.k3s.kubectl(&arguments).await.unwrap_or_else(|error| panic!("kubectl {step:?}: {error}"));
    }
    let dangling = ["wait", "--for=condition=Dangling", "flowschema/fresnel-dangling", LIMIT];
    cluster.k3s.kubectl(&dangling).await.unwrap_or_else(|error| panic!("the FlowSchema with no level: {error}"));
    started.elapsed()
}
