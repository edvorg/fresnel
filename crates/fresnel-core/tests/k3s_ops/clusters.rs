//! Weakness 9 of plan section 6: an operation reaches the cluster of its object, and no
//! other one, also when two clusters have the same objects.

use fresnel_core::kinds::Kind;
use fresnel_core::ops::{Cancel, JobState, Operation, Order};
use serde_json::Value as Json;

use crate::bed::{APP, Bed, Cluster, normal, run_parts};

/// The objects of the scene that an operation of the test changes or removes.
const CHANGED: [(Kind, &str); 13] = [
    (Kind::Deployment, "web"),
    (Kind::Deployment, "paused"),
    (Kind::Deployment, "guarded"),
    (Kind::StatefulSet, "db"),
    (Kind::DaemonSet, "agent"),
    (Kind::ReplicaSet, "set"),
    (Kind::ReplicaSet, "orphans"),
    (Kind::ReplicationController, "legacy"),
    (Kind::Job, "once"),
    (Kind::Job, "doomed"),
    (Kind::CronJob, "nightly"),
    (Kind::Pod, "lone"),
    (Kind::Pod, "evictee"),
];

/// The state of each object that the test changes in the namespace of the app, and the
/// names of its Jobs: a Job that a trigger made would be new.
pub(crate) async fn state(cluster: &Cluster) -> (Vec<Option<Json>>, Vec<String>) {
    let mut objects = Vec::new();
    for (kind, name) in CHANGED {
        objects.push(cluster.object(APP, kind, name).await.map(normal));
    }
    let jobs = cluster.kubectl(&["get", "jobs.batch", "--namespace", APP, "--output", "name"]).await;
    (objects, jobs.lines().map(str::to_owned).collect())
}

/// An order for an object of `b` with the gateway of `a` goes nowhere: each request carries
/// the identity of its cluster (design rule 8).
pub(crate) async fn an_order_goes_to_the_cluster_of_its_object_only(bed: &Bed) {
    let order = Order::new(bed.b.target(Kind::Deployment, "web").await, Operation::Scale(5)).expect("an order");
    let report = run_parts(&order, bed.a.parts(Cancel::pair().1)).await;
    let JobState::NotSent(error) = &report.state else { panic!("not sent: {report:#?}") };
    assert_eq!(error.to_string(), "a request for b cannot go to a");
    assert!(report.steps.is_empty(), "no step started");
    for cluster in [&bed.a, &bed.b] {
        let web = cluster.object(APP, Kind::Deployment, "web").await.expect("the Deployment");
        assert_eq!(web.pointer("/spec/replicas"), Some(&Json::from(1)), "{}", cluster.id);
    }
}
