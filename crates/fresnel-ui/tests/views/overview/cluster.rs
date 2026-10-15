//! The Cluster overview: a block for each selected cluster with the values of its own
//! stores and sampler (weaknesses 1 to 4 of plan section 6), and the links of its counts.

use std::collections::BTreeSet;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::{ClusterRow, Command};
use fresnel_core::cluster::{LinkState, ServerVersion, VersionState};
use fresnel_core::kinds::Kind;
use fresnel_core::metrics::PollState;
use fresnel_core::store::StoreState;

use super::{cluster, open};
use crate::data::{held, in_state};
use crate::details::shows_part;
use crate::fake::{SECOND, error, now, row, shows, until, view, window};
use crate::kinds::in_panel;
use crate::pods::scenes::no_answer;

/// Weaknesses 1 and 2: two clusters with a node of the same name, each block with the
/// version, the nodes, the pods, the usage and the warnings of its own cluster.
#[test]
fn each_selected_cluster_has_a_block_with_its_version_nodes_pods_usage_and_warnings() {
    let mut window = window(view(vec![cluster("ams-prod", 1, "v1.33.5+k3s1"), cluster("fra-prod", 2, "v1.32.9+k3s1")]));
    open(&mut window, "Cluster overview");
    let wanted = BTreeSet::from([Kind::Node, Kind::Event]);
    assert_eq!(window.state().sent.last(), Some(&Command::ShowKinds(wanted)), "the stores of the counts");
    for text in ["v1.33.5+k3s1", "v1.32.9+k3s1"] {
        assert!(shows(&window, text), "{text}");
    }
    assert_eq!(window.query_all_by_label("1 of 1 node ready").count(), 2);
    assert_eq!(window.query_all_by_label("25 pods").count(), 2);
    for phase in ["· 12 running", "· 6 pending", "· 4 succeeded", "· 3 failed"] {
        assert_eq!(window.query_all_by_label(phase).count(), 2, "{phase}");
    }
    assert!(!shows(&window, "· 0 unknown"), "a phase with no pod");
    // The sum of the node against its 18 allocatable cores, and the requests on it.
    assert!(shows_part(&window, "1.5 of 18 allocatable (8%) · requests 2.5 · sample 5 s ago"), "ams");
    assert!(shows_part(&window, "3 of 18 allocatable (17%) · requests 5 · sample 5 s ago"), "fra");
    assert_eq!(window.query_all_by_label("31 warnings").count(), 2);
    assert!(shows(&window, "BackOff") && shows(&window, "Pod crash"), "the newest warnings with their objects");
    assert!(!shows_part(&window, "old") && !shows_part(&window, "not loaded"), "each store is live");
}

/// The block of a cluster in this state: its pods in a pause, no metrics-server, a failed
/// read of its version and a cordoned node that is not ready.
pub(super) fn degraded(name: &str) -> ClusterRow {
    let row = cluster(name, 2, "v1.32.9+k3s1");
    let mut data = row.data.clone().expect("data");
    let pause = StoreState::Backoff { failures: 2, reason: no_answer(), until: now() + 2 * SECOND };
    data.pods = held(in_state(&data.pods, pause));
    let mut metrics = fresnel_core::metrics::Metrics::clone(&data.metrics);
    let gone = error("the server could not find the requested resource (HTTP 404)");
    (metrics.nodes.state, metrics.nodes.since) = (PollState::NotInstalled(gone), now() - 30 * SECOND);
    data.metrics = held(metrics);
    let failed = VersionState::Failed { error: error("no answer in 10 s"), at: now() - 10 * SECOND };
    data.version = held(ServerVersion { version: Some("v1.32.9+k3s1".into()), state: failed });
    ClusterRow { data: Some(data), ..row }
}

/// Weaknesses 3 and 4 and design rule 5: each cluster that is not live shows its own state
/// with its old values, and the block of a live cluster keeps each value.
#[test]
fn a_cluster_whose_stores_or_sampler_are_not_live_shows_its_state_and_the_others_keep_their_values() {
    let failed = row("lab", LinkState::Failed(error("the cluster entry `lab` has no certificate authority")));
    let mut window = window(view(vec![cluster("ams-prod", 1, "v1.33.5+k3s1"), degraded("fra-prod"), failed]));
    open(&mut window, "Cluster overview");
    assert!(shows_part(&window, "1.5 of 18 allocatable (8%)"), "ams keeps its values");
    let old_version = "v1.32.9+k3s1, old: the read of 07:59:50 failed: no answer in 10 s";
    assert!(shows(&window, old_version), "the version of before, with the reason");
    assert!(shows_part(&window, "3 of 18 allocatable (17%)"), "the old value of fra");
    let state = "NodeMetrics: metrics-server not found: the server could not find the requested resource (HTTP 404)";
    assert!(shows_part(&window, state), "the state of the sampler of fra");
    assert_eq!(window.query_all_by_label("old since 07:59:12").count(), 1, "the pods of fra");
    assert!(shows(&window, "The connect did not start: the cluster entry `lab` has no certificate authority"));
    assert_eq!(window.query_all_by_label("25 pods").count(), 2, "the old count of fra stays");
}

#[test]
fn a_count_opens_its_table_with_the_problems_filter_and_a_warning_opens_its_object() {
    let mut window = window(view(vec![cluster("ams-prod", 1, "v1.33.5+k3s1")]));
    open(&mut window, "Cluster overview");
    let problems =
        window.query_all_by_label_contains(" with problems").find(|node| node.accesskit_node().role() == Role::Link);
    let text = problems.expect("the link of the pods with problems").accesskit_node().label().expect("a text");
    window.get_by_role_and_label(Role::Link, &text).click();
    let count = text.trim_end_matches(" with problems").to_owned();
    until(&mut window, "the pods with problems", |window| shows(window, &format!("{count} of 25 pods")));

    window.get_by_role_and_label(Role::RadioButton, "Cluster overview").click();
    window.run();
    open(&mut window, "Cluster overview");
    window.get_by_role_and_label(Role::Link, "31 warnings").click();
    until(&mut window, "the warnings", |window| shows(window, "31 of 196 Events"));

    open(&mut window, "Cluster overview");
    window
        .query_all_by_label("Pod crash")
        .find(|node| node.accesskit_node().role() == Role::Link)
        .expect("a link")
        .click();
    until(&mut window, "the panel of the pod", |window| in_panel(window, "CONTAINERS") && in_panel(window, "crash"));
}
