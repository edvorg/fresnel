//! The log streams on a real kube-apiserver (plan section 15, M9): k3s in Docker behind the
//! fault proxy, with the stores and the owner index of the app.
//!
//! - `bed`: the cluster, the app before it and the views. `memory`: the bytes of the process.
//! - `restart`: a stream that breaks starts again and loses no line, with a marker.
//! - `previous`: the log of the instance of a container before its crash.
//! - `workload`: a container that starts again and a new pod in the view of a Deployment.
//! - `million`: 1,000,000 lines keep the memory of a view bounded.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod bed;
#[path = "../common/mod.rs"]
mod common;
mod memory;
mod million;
mod previous;
mod restart;
#[path = "../common/stores.rs"]
mod store_bed;
mod workload;

use bed::Bed;

/// The count of the heap of this test binary: the measure of `million`.
#[global_allocator]
static HEAP: memory::Counting = memory::Counting;

#[tokio::test]
#[ignore = "needs k3s in Docker"]
async fn each_log_view_on_k3s_loses_no_line_and_keeps_its_limits() {
    let bed = Bed::start().await;
    restart::a_break_of_each_connection_loses_no_line_and_shows_one_marker(&bed).await;
    previous::the_earlier_instance_of_a_crashed_container_has_its_lines(&bed).await;
    workload::a_container_that_starts_again_and_a_new_pod_stay_in_the_view_of_the_deployment(&bed).await;
    million::one_million_lines_keep_the_memory_of_a_view_bounded(&bed).await;
}
