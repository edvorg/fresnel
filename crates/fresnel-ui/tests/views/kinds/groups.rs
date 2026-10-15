//! The table and the detail panel of the kind of each group of the sidebar (plan section
//! 11.2): the columns of the kind with the Cluster column, the metadata, the conditions,
//! the related objects as links from the indexes, and the Events.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::{ClusterRow, Command};
use fresnel_core::kinds::{Describe, Kind};
use fresnel_core::store::StoreState;
use k8s_openapi::api::apps::v1::{Deployment, ReplicaSet};

use super::scene::{config_maps, deployments, live, nodes, revisions, service_accounts, services, with};
use super::weakness::{count_of, page};
use super::{in_panel, times_in_panel};
use crate::data::{changed_object, empty_store, held, pods, scene, store};
use crate::fake::{SECOND, Window, absent, now, shows, until, view};
use crate::pods::column;

/// Opens the panel of the object of the row with this name.
fn open(window: &mut Window, name: &str) {
    window.get_by_label(name).click();
    window.run();
    assert!(shows(window, "Close the details"), "the panel of {name}");
}

/// Follows a link of the panel. The panel scrolls to it first: a click needs it on the screen.
fn follow(window: &mut Window, link: &str) {
    let is_link = |node: &egui_kittest::Node<'_>| node.accesskit_node().role() == egui::accesskit::Role::Link;
    window.query_all_by_label(link).find(is_link).unwrap_or_else(|| panic!("the link {link}")).scroll_to_me();
    window.run();
    window.query_all_by_label(link).find(is_link).unwrap_or_else(|| panic!("the link {link}")).click();
    window.run();
}

/// The names of the pods of the scene that start with this text.
fn pod_names(start: &str) -> Vec<String> {
    let names = pods().into_iter().filter_map(|pod| pod.metadata.name.clone());
    names.filter(|name| name.starts_with(start)).collect()
}

#[test]
fn a_node_shows_its_conditions_with_no_namespace_and_a_link_of_a_pod_on_it_opens_the_pod() {
    let nodes = nodes();
    let name = nodes[0].metadata.name.clone().expect("a name");
    let mut window = page::<k8s_openapi::api::core::v1::Node>(vec![live("ams-prod", &nodes)], 1);
    assert_eq!(column(&window, "Status"), ["Ready"]);
    assert_eq!(window.state().sent, [Command::ShowKinds(BTreeSet::from([Kind::Node]))]);
    open(&mut window, &name);
    let texts = ["Node", "Healthy", "METADATA", "CONDITIONS", "MemoryPressure", "Ready", "PODS", "db-0", "EVENTS"];
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
    let namespaces = times_in_panel(&window, "Namespace");
    assert_eq!(namespaces, 1, "a node has no namespace: the one in the panel is the column of its pods");

    // The link opens the Pods with the panel of the pod, and the core holds no Nodes for it.
    follow(&mut window, "db-0");
    // The panel reads the snapshot, and the count waits for the rows of the worker thread.
    until(&mut window, "the panel of the pod on the page of the Pods", |window| {
        in_panel(window, "db-0") && in_panel(window, "Pod") && shows(window, "25 pods")
    });
    assert!(in_panel(&window, "StatefulSet db"));
    let shown = window.state().sent.iter().rev().find(|command| matches!(command, Command::ShowKinds(_))).cloned();
    assert_eq!(shown, Some(Command::ShowKinds(BTreeSet::new())));
}

#[test]
fn a_deployment_shows_its_replica_sets_and_its_pods_and_the_links_go_down_and_up() {
    let mut data = live("ams-prod", &deployments()).data.expect("data");
    data.objects = data.objects.with(held(store(StoreState::Live, &scene::<ReplicaSet>())));
    let cluster = ClusterRow { data: Some(data), ..live("ams-prod", &deployments()) };
    let mut window = page::<Deployment>(vec![cluster], 4);
    assert_eq!(column(&window, "Name"), ["broken", "coredns", "local-path-provisioner", "web"]);
    assert_eq!(column(&window, "Ready"), ["0/1", "1/1", "1/1", "2/2"]);
    open(&mut window, "web");
    let mut texts = vec!["Deployment", "CONDITIONS", "Available", "MinimumReplicasAvailable", "REPLICASETS", "PODS"];
    texts.extend(["ReplicaSet web-64cb9bd5d4", "ReplicaSet web-6888b778b6", "EVENTS"]);
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
    for pod in pod_names("web-") {
        assert!(in_panel(&window, &pod), "the pod {pod}");
    }

    follow(&mut window, "ReplicaSet web-6888b778b6");
    let replica_sets = count_of::<ReplicaSet>(scene::<ReplicaSet>().len());
    until(&mut window, &format!("the panel of the ReplicaSet and {replica_sets}"), |window| {
        in_panel(window, "Deployment web") && shows(window, &replica_sets)
    });
    assert!(in_panel(&window, "ReplicaSet"));
    follow(&mut window, "Deployment web");
    until(&mut window, "the panel of the Deployment and 4 Deployments", |window| {
        in_panel(window, "REPLICASETS") && shows(window, "4 Deployments")
    });
}

#[test]
fn a_config_map_shows_its_keys_and_the_pods_that_use_it() {
    let mut window = page::<k8s_openapi::api::core::v1::ConfigMap>(vec![live("ams-prod", &config_maps())], 2);
    assert_eq!(column(&window, "Keys"), ["ca.crt", "color, greeting, logo"], "the keys of data and binaryData");
    open(&mut window, "web-config");
    assert!(in_panel(&window, "USED BY") && in_panel(&window, "Healthy"));
    for pod in pod_names("web-6888b778b6-") {
        assert!(in_panel(&window, &pod), "the pod {pod}");
    }
}

#[test]
fn a_service_shows_its_endpoint_slices_and_the_pods_behind_it() {
    let mut window = page::<k8s_openapi::api::core::v1::Service>(vec![live("ams-prod", &services())], 8);
    open(&mut window, "web");
    assert_eq!(absent(&window, &["ENDPOINTSLICES", "EndpointSlice web-5qcnf", "ENDPOINTS"]), Vec::<&str>::new());
    for pod in pod_names("web-6888b778b6-") {
        assert!(in_panel(&window, &pod), "the pod {pod}");
    }
    open(&mut window, "nobody");
    assert!(in_panel(&window, "None."), "a Service with no pod, and a live index: no pod");
}

#[test]
fn a_service_account_shows_the_pods_that_use_it() {
    let mut window = page::<k8s_openapi::api::core::v1::ServiceAccount>(vec![live("ams-prod", &service_accounts())], 2);
    open(&mut window, "web");
    assert!(in_panel(&window, "USED BY"));
    for pod in pod_names("web-6888b778b6-") {
        assert!(in_panel(&window, &pod), "the pod {pod}");
    }
}

/// Plan section 11.2: a kind with no sections of its own shows its metadata and its Events.
#[test]
fn a_kind_with_the_generic_view_shows_its_metadata_its_owner_and_its_events() {
    let revisions = revisions();
    let mut window = page::<k8s_openapi::api::apps::v1::ControllerRevision>(vec![live("ams-prod", &revisions)], 2);
    assert_eq!(column(&window, "Controller"), ["DaemonSet/agent", "StatefulSet/db"]);
    open(&mut window, "db-64d4d44cf9");
    let texts = ["ControllerRevision", "METADATA", "Namespace", "UID", "Created", "StatefulSet db", "No Events."];
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
    assert!(!in_panel(&window, "CONDITIONS") && !in_panel(&window, "USED BY"), "no section of an other kind");
}

/// Plan section 11.2: the panel builds its data again for each new snapshot, and a deleted
/// object shows as deleted. A store with no complete list does not know its objects.
#[test]
fn the_panel_shows_each_new_version_and_a_deleted_object_as_deleted() {
    let all = deployments();
    let mut window = page::<Deployment>(vec![live("ams-prod", &all)], 4);
    open(&mut window, "web");
    assert!(in_panel(&window, "MinimumReplicasAvailable"));

    let web = all.iter().find(|object| object.metadata.name.as_deref() == Some("web")).expect("web");
    let lost = changed_object(&**web, |value| {
        value["status"]["conditions"][0]["reason"] = "MinimumReplicasUnavailable".into();
        value["metadata"]["labels"] = serde_json::json!({ "tier": "front" });
    });
    let others = all.iter().filter(|object| !Arc::ptr_eq(object, web)).cloned();
    let next: Vec<Arc<Deployment>> = others.clone().chain([lost]).collect();
    window.state_mut().view = view(vec![live("ams-prod", &next)]);
    window.step();
    assert!(in_panel(&window, "MinimumReplicasUnavailable") && in_panel(&window, "tier=front"), "the new version");

    let gone: Vec<Arc<Deployment>> = others.collect();
    window.state_mut().view = view(vec![live("ams-prod", &gone)]);
    window.step();
    assert!(shows(&window, "This Deployment is deleted. The panel shows its last state."));
    assert!(in_panel(&window, "tier=front"), "the last state stays");
    until(&mut window, "the rows with no web", |window| shows(window, &count_of::<Deployment>(3)));

    let listing = empty_store::<Deployment>(StoreState::Listing { loaded: 0 }, now() - SECOND);
    window.state_mut().view = view(vec![with("ams-prod", listing)]);
    window.step();
    let text = "The Deployments of ams-prod are not loaded now. The panel shows the last state of this Deployment.";
    assert!(shows(&window, text) && !shows(&window, "This Deployment is deleted. The panel shows its last state."));
    assert_eq!(Kind::Deployment, Deployment::ID);
}

/// Extreme sizes: a window from 1 point to 4,000 points wide, with the panel and its
/// Manifest tab open. Each size settles in a few frames with no panic, and the panel stays.
#[test]
fn the_page_with_the_panel_and_the_manifest_tab_draws_in_a_window_of_each_size() {
    let mut window = page::<Deployment>(vec![live("ams-prod", &deployments())], 4);
    open(&mut window, "web");
    window.get_by_label("Manifest").click();
    window.run();
    let sizes = [egui::vec2(300.0, 200.0), egui::vec2(120.0, 80.0), egui::vec2(1.0, 1.0), egui::vec2(4000.0, 3000.0)];
    for size in sizes.into_iter().chain([crate::fake::WINDOW]) {
        window.set_size(size);
        // `run` fails when the frames ask for more frames with no end.
        window.run();
    }
    assert!(shows(&window, "Close the details") && shows(&window, "The manifest is read from ams-prod…"));
}
