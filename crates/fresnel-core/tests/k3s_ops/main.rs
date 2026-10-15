//! The operations on a real kube-apiserver (plan section 15, M3): two k3s clusters in Docker
//! with the same objects. Each operation of the app on an object of cluster `b` gives the
//! state that the same kubectl command gives its twin in an other namespace, and cluster
//! `a` does not change (weakness 9 of plan section 6).
//!
//! - `bed`: the two clusters, the run of an order and the comparison with the twin.
//! - `scene`: the objects. `clusters`: the identity of the cluster of each request.
//! - `workloads`: scale, restart, rollout status, history and rollback. `removal`: delete,
//!   force delete and evict. `batch`: suspend, resume, trigger and run again.
//! - `access`: the permissions of a user with a Role, and HTTP 403 of a real request.

// k3s in Docker: the cluster job of CI runs on Linux.
#![cfg(unix)]
#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod access;
mod batch;
mod bed;
mod clusters;
#[path = "../common/mod.rs"]
mod common;
mod removal;
mod scene;
#[path = "../common/stores.rs"]
#[expect(dead_code, reason = "this test reads no index of the stores")]
mod store_bed;
#[path = "../common/user.rs"]
mod user;
mod workloads;

use bed::Bed;

#[tokio::test]
#[ignore = "needs k3s in Docker"]
async fn each_operation_gives_the_state_of_kubectl_and_reaches_its_own_cluster_only() {
    let bed = Bed::start().await;
    tokio::join!(scene::create(&bed.a), scene::create(&bed.b));
    let before = clusters::state(&bed.a).await;
    clusters::an_order_goes_to_the_cluster_of_its_object_only(&bed).await;

    workloads::scale(&bed.b).await;
    workloads::restart(&bed.b).await;
    workloads::rollout_status(&bed.b).await;
    workloads::history(&bed.b).await;
    workloads::rollback(&bed.b).await;
    removal::precondition(&bed.b).await;
    removal::evict(&bed.b).await;
    removal::force_delete(&bed.b).await;
    removal::delete(&bed.b).await;
    batch::suspend_and_resume(&bed.b).await;
    batch::trigger(&bed.b).await;
    batch::run_again(&bed.b).await;
    access::a_forbidden_action_is_denied_with_its_reason_and_its_request_fails(&bed).await;

    assert!(clusters::state(&bed.a).await == before, "an operation on b changed cluster a");
}
