//! Port forwards through the real core on k3s in Docker behind the fault proxy (plan
//! sections 12.3 and 15, M5): a forward to each kind of target, a pod that restarts, an
//! upload of 200 MB to a pod that reads nothing for 30 s, a reset storm and held connects
//! with a client of each second, a break of the network and a wake.
//!
//! - `bed`: the cluster with its scene (`scene.yaml`), the proxy and the core.
//! - `steps`: the steps of the test, in the sequence of their faults.

// k3s in Docker: the cluster job of CI runs on Linux.
#![cfg(unix)]
#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod bed;
#[path = "../common/mod.rs"]
#[expect(dead_code, reason = "this test waits for the view of the core, not for one signal of a cluster")]
mod common;
#[path = "../common/contexts.rs"]
mod contexts;
#[path = "../common/gaps.rs"]
mod gaps;
mod steps;

use bed::Bed;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs k3s in Docker"]
async fn forwards_reach_each_kind_of_target_and_recover_after_each_break_on_k3s() {
    let bed = Bed::start().await;
    let forwards = steps::each_kind_of_target_reaches_a_pod_of_its_object(&bed).await;
    steps::a_forward_works_again_5_s_or_less_after_its_pod_restarts(&bed).await;
    steps::an_upload_of_200_mb_to_a_pod_that_reads_nothing_for_30_s_completes(&bed).await;
    // The faults of the network come last: each one breaks the connections of the cluster.
    steps::each_run_of_failed_pages_in_a_reset_storm_and_held_connects_shows_a_state(&bed).await;
    steps::the_local_ports_stay_open_in_a_break_and_work_again_10_s_or_less_after_it(&bed, &forwards).await;
    steps::each_forward_works_again_10_s_or_less_after_a_wake(&bed, &forwards).await;
}
