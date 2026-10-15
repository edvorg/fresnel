//! The commands of the views through the real app core on two k3s clusters in Docker (plan
//! sections 7.3, 9.10 and 10). The tests of `k3s_ops` call the operations directly, and the
//! view tests use a fake core: here each command goes the way of the window, from
//! `Core::send` to the view of the core, and kubectl reads each cluster.
//!
//! - `bed`: the two clusters, a user of cluster b with a Role, and the core with the
//!   contexts `a`, `b` and `viewer`. `scene`: the objects of the test.
//! - `jobs`: `Command::Run` for scale, rollback, restart, delete and trigger, each to the
//!   final state of its job and the mark "change pending"; `Command::Cancel` of a rollout status.
//! - `keys`: the save of a key of a Secret. `access`: `Command::ReadRules` and
//!   `Command::CheckAccess` for the admin and for the user who may not scale.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod access;
mod bed;
#[path = "../common/mod.rs"]
#[expect(dead_code, reason = "each wait of this test reads the view of the core, not one signal")]
mod common;
#[path = "../common/contexts.rs"]
mod contexts;
mod jobs;
mod keys;
mod scene;
#[path = "../common/user.rs"]
mod user;

use bed::Bed;

/// Weakness 9 of plan section 6 in the real core: each order reaches the cluster of its
/// object, and cluster a does not change after the orders for b.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs k3s in Docker"]
async fn each_command_of_a_view_runs_in_the_core_and_reaches_the_cluster_of_its_object() {
    let bed = Bed::start().await;
    jobs::scale_in_each_cluster(&bed).await;
    let before = scene::state(&bed.a).await;
    jobs::rollback(&bed).await;
    jobs::restart(&bed).await;
    jobs::delete(&bed).await;
    jobs::trigger(&bed).await;
    jobs::cancel_ends_a_rollout_status(&bed).await;
    keys::save_a_key_of_a_secret(&bed).await;
    access::the_rules_and_the_reviews_of_each_context_decide_its_actions(&bed).await;
    assert!(scene::state(&bed.a).await == before, "an order for b changed cluster a");
    bed.no_task_failed();
}
