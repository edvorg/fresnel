//! The Workloads overview: the objects of each workload kind by health in each selected
//! cluster, the namespace filter, the links to the tables and a store that is not loaded.

use std::collections::BTreeSet;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::{ClusterRow, Command};
use fresnel_core::kinds::Kind;
use fresnel_core::store::StoreState;
use k8s_openapi::api::core::v1::ReplicationController;

use super::{cluster, open};
use crate::data::{empty_store, held};
use crate::details::shows_part;
use crate::fake::{absent, now, shows, until, view, window};

/// The count of a kind in a cluster for a screen reader: "Deployments of ams-prod: 1 failed".
fn link(window: &crate::fake::Window, text: &str) -> bool {
    window.query_all_by_label(text).any(|node| node.accesskit_node().role() == Role::Link)
}

/// A cluster whose store of the ReplicationControllers did not list yet.
fn listing(name: &str) -> ClusterRow {
    let row = cluster(name, 1, "v1.33.5+k3s1");
    let mut data = row.data.clone().expect("data");
    let first = StoreState::Listing { loaded: 0 };
    data.objects = data.objects.with(held(empty_store::<ReplicationController>(first, now())));
    ClusterRow { data: Some(data), ..row }
}

#[test]
fn each_workload_kind_counts_by_health_in_each_cluster_and_a_count_of_problems_opens_its_table() {
    let mut window = window(view(vec![cluster("ams-prod", 1, "v1.33.5+k3s1"), listing("fra-prod")]));
    open(&mut window, "Workloads overview");
    let kinds = [
        Kind::Deployment,
        Kind::StatefulSet,
        Kind::DaemonSet,
        Kind::ReplicaSet,
        Kind::ReplicationController,
        Kind::Job,
        Kind::CronJob,
    ];
    assert_eq!(window.state().sent.last(), Some(&Command::ShowKinds(BTreeSet::from(kinds))));
    for kind in [Kind::Pod].into_iter().chain(kinds) {
        assert!(link(&window, kind.info().title), "the title of {kind:?} links to its table");
    }
    assert!(link(&window, "Deployments of ams-prod: 1 failed") && link(&window, "Deployments of fra-prod: 1 failed"));
    assert!(link(&window, "Jobs of ams-prod: 1 failed"), "the Job fails");
    let not_loaded = "not loaded: the store list in progress, 0 objects";
    assert!(shows(&window, not_loaded), "the ReplicationControllers of fra");
    assert!(!link(&window, "Pods of ams-prod: 0 failed"), "a count of zero is no link");

    window.get_by_label("Deployments of fra-prod: 1 failed").click();
    until(&mut window, "the Deployments with problems", |window| shows(window, "2 of 8 Deployments"));
    assert!(shows(&window, "broken"), "the Deployment of the count");
}

/// The counts follow the namespace filter of the top bar, as the tables do: a link opens a
/// table whose rows are the objects of the count. A new filter and new clusters in the
/// frames after it, with no wait for the worker, end with the counts of both.
#[test]
fn the_counts_follow_the_namespace_filter_and_the_last_clusters() {
    let mut window = window(view(vec![cluster("ams-prod", 1, "v1.33.5+k3s1")]));
    open(&mut window, "Workloads overview");
    assert!(shows(&window, "Pods of ams-prod: 25 objects") && link(&window, "Pods of ams-prod: 7 failed"));
    window.get_by_label("All namespaces").click();
    window.run();
    window.get_by_role_and_label(Role::CheckBox, "kube-system").click();
    window.state_mut().view = view(vec![cluster("fra-prod", 1, "v1.33.5+k3s1")]);
    window.step();
    window.key_press(egui::Key::Escape);
    until(&mut window, "the counts of kube-system", |window| shows(window, "Pods of fra-prod: 2 objects"));
    assert!(shows(&window, "The objects of the namespaces kube-system only: the namespace filter."));
    let counts =
        ["Deployments of fra-prod: 2 objects", "Deployments of fra-prod: 0 failed", "Jobs of fra-prod: 0 objects"];
    assert_eq!(absent(&window, &counts), Vec::<&str>::new());
    assert!(!shows_part(&window, "ams-prod"), "the cluster that left the selection");
}
