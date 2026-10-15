//! Cordon, uncordon and drain of a node through the real app core on k3s in Docker (plan
//! sections 10 and 15, M7): each command goes the way of the window, from `Core::send` to
//! the view of the core, and kubectl reads the cluster. The app drains one node, kubectl
//! its twin with the same options, and the two nodes end with the same pods.
//!
//! - `bed`: cluster `a`, and cluster `b` with two agents; the core with the contexts.
//! - `scene`: the pods on each node. `steps`: the cordon, a drain that pods block, a budget
//!   that holds a pod until the time limit, a cancel, and a drain that ends.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod bed;
#[path = "../common/mod.rs"]
#[expect(dead_code, reason = "each wait of this test reads the view of the core, not one signal")]
mod common;
#[path = "../common/contexts.rs"]
mod contexts;
mod scene;
mod steps;

use bed::{Bed, DRAINED, TWIN};

/// Weakness 9 of plan section 6: cluster `a` has a node with the name of the drained one,
/// and its node and pods do not change.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs k3s in Docker"]
async fn a_drain_through_the_core_leaves_the_state_of_kubectl_drain_and_reaches_its_own_cluster_only() {
    let bed = Bed::start().await;
    let both = [("app", DRAINED), ("kubectl", TWIN)];
    tokio::join!(scene::create(&bed.a, &both[..1], false), scene::create(&bed.b, &both, true));
    let other = bed.a.pods_on(DRAINED).await;
    steps::cordon_and_uncordon(&bed).await;
    steps::a_pod_that_blocks_stops_the_drain_before_its_first_eviction(&bed).await;
    steps::a_budget_holds_a_pod_by_its_name_until_the_time_limit(&bed).await;
    steps::a_cancel_stops_the_drain_and_the_node_stays_cordoned(&bed).await;
    steps::with_no_budget_the_drain_ends_as_kubectl_drain(&bed).await;
    assert_eq!(bed.a.pods_on(DRAINED).await, other, "an order for b changed the pods of a");
    assert!(!bed.a.cordoned(DRAINED).await, "an order for b cordoned the node of a");
}
