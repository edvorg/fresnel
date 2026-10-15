//! The metrics sampler on real kube-apiservers (plan sections 12.4 and 15, M6): three k3s
//! clusters in Docker. `a` and `b` run metrics-server and have a node of the same name, `c`
//! has no metrics-server. The samples of each series arrive (`samples`), two clusters with
//! a node of the same name keep their own series (weakness 2 of plan section 6), the
//! cluster with no metrics API shows that state while the others keep their values
//! (weakness 4), and the history keeps its limits (`history`).
//!
//! - `bed`: the clusters, their stores and a sampler of each one.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod bed;
#[path = "../common/mod.rs"]
#[expect(dead_code, reason = "the test connects three clusters of its own registry")]
mod common;
mod history;
mod samples;
#[path = "../common/stores.rs"]
mod store_bed;

use fresnel_core::metrics::{Metrics, SeriesKey};

use bed::{Bed, NODE, until};

#[tokio::test]
#[ignore = "needs k3s in Docker"]
async fn the_metrics_of_each_cluster_arrive_stay_its_own_and_keep_their_limits() {
    let bed = Bed::start().await;
    samples::of_each_series_arrive(&bed.a).await;
    two_clusters_with_a_node_of_the_same_name_keep_their_own_values(&bed).await;
    a_cluster_with_no_metrics_api_shows_it_and_the_others_keep_their_values(&bed).await;
    history::the_memory_limit_holds_and_the_containers_lose_first(&bed).await;
    history::the_tiers_keep_their_time_and_the_series_of_a_deleted_pod_go(&bed).await;
}

/// Weakness 2: a match by name without the cluster. The node of `a` runs a pod that uses
/// CPU, the node of `b` runs none, so the two values differ.
async fn two_clusters_with_a_node_of_the_same_name_keep_their_own_values(bed: &Bed) {
    let node = SeriesKey::Node(NODE.into());
    let cpu = |metrics: &Metrics, cluster| metrics.latest(cluster, &node).filter(|latest| latest.current);
    let of_b = until(&bed.b.metrics, "a value of the node of b", |metrics| cpu(metrics, &bed.b.id).is_some()).await;
    let busier = |metrics: &Metrics| {
        let b = cpu(&bed.b.metrics.read(), &bed.b.id).map(|latest| latest.sample.usage.cpu);
        let a = cpu(metrics, &bed.a.id).map(|latest| latest.sample.usage.cpu);
        a.zip(b).is_some_and(|(a, b)| a > b + 100_000_000)
    };
    let of_a = until(&bed.a.metrics, "the node of a uses 100m more CPU than the node of b", busier).await;
    assert_eq!(of_a.latest(&bed.b.id, &node), None, "a lookup of b in the metrics of a");
    assert_eq!(of_b.latest(&bed.a.id, &node), None, "a lookup of a in the metrics of b");
    assert_eq!(of_a.node_load(&bed.b.id, NODE), None);
    let load = |metrics: &Metrics, cluster| metrics.node_load(cluster, NODE).map(|load| load.requests.cpu);
    assert!(load(&of_a, &bed.a.id) > load(&of_b, &bed.b.id), "the pod of a requests 100m");
}

/// Weakness 4: an error of one cluster changes the view of all clusters.
async fn a_cluster_with_no_metrics_api_shows_it_and_the_others_keep_their_values(bed: &Bed) {
    let not_installed = |metrics: &Metrics| {
        let state = |status: &fresnel_core::metrics::PollStatus| {
            matches!(status.state, fresnel_core::metrics::PollState::NotInstalled(_))
        };
        state(&metrics.pods) && state(&metrics.nodes)
    };
    let c = until(&bed.c.metrics, "no metrics API in c", not_installed).await;
    assert_eq!(c.series_count(), 0, "no value, and no zero");
    assert!(c.pods.state.to_string().starts_with("metrics-server not found: "), "{}", c.pods.state);
    let node = SeriesKey::Node(NODE.into());
    for cluster in [&bed.a, &bed.b] {
        let before = cluster.metrics.read().nodes.answers;
        let later = |metrics: &Metrics| {
            let current = metrics.latest(&cluster.id, &node).is_some_and(|latest| latest.current);
            metrics.nodes.answers >= before + 2 && metrics.pods.state.is_ok() && current
        };
        until(&cluster.metrics, "two more answers while c has no metrics API", later).await;
    }
    assert!(not_installed(&bed.c.metrics.read()), "c keeps its state");
}
