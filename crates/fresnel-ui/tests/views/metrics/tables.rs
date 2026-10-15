//! The CPU and memory columns of the tables of the Pods, the Nodes and the workloads: the
//! value with its request and limit and the age of its sample, an old value, a value that
//! is missing, and the weaknesses 2 and 4 of plan section 6.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::ClusterRow;
use fresnel_core::cluster::ClusterId;
use fresnel_core::metrics::{Metrics, PollState};
use k8s_openapi::api::apps::v1::Deployment;

use super::{LEAVING, NODE, failed, of_kind, of_pods, scene_metrics};
use crate::data::{held, scene};
use crate::fake::{SECOND, Window, error, now, open_page, until, view, window};
use crate::kinds::scene::nodes;
use crate::pods::column;

/// A window wide enough for each column of the Pods, after the age.
pub(super) fn wide(clusters: Vec<ClusterRow>) -> Window {
    let mut window = window(view(clusters));
    window.set_size(egui::vec2(1900.0, 900.0));
    window.run();
    window
}

/// Shows only the rows with this text, so the rows that a test reads are on the screen.
fn filter(window: &mut Window, text: &str) {
    window.get_by_role(Role::TextInput).click();
    window.run();
    window.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    window.get_by_role(Role::TextInput).type_text(text);
    window.run();
}

/// The words of the cells that have this text, for the pointer and a screen reader.
pub(super) fn hints(window: &Window, text: &str) -> Vec<String> {
    let nodes = window.query_all_by_label_contains(text);
    nodes.filter_map(|node| node.accesskit_node().value()).collect()
}

pub(super) fn until_hint(window: &mut Window, text: &str) {
    until(window, &format!("a cell with {text:?}"), |window| !hints(window, text).is_empty());
}

#[test]
fn a_pod_shows_its_usage_with_its_request_its_limit_and_the_age_of_its_sample() {
    let mut window = wide(vec![of_pods("ams", scene_metrics("ams", 1))]);
    filter(&mut window, "web-");
    let cpu = "CPU 460m\nrequest 100m (460%)\nlimit 500m (92%)\nsample of 07:59:55, 5 s ago";
    until_hint(&mut window, cpu);
    let memory = "Memory 120Mi\nrequest 64Mi (188%)\nlimit 128Mi (94%)\nsample of 07:59:55, 5 s ago";
    assert_eq!(hints(&window, "Memory 120Mi"), [memory]);
    assert_eq!(column(&window, "CPU").len(), 2, "the two pods of web: {:?}", column(&window, "CPU"));
}

#[test]
fn a_value_that_the_last_answer_did_not_have_is_old_and_a_pod_with_no_sample_has_none() {
    let mut window = wide(vec![of_pods("ams", scene_metrics("ams", 1))]);
    filter(&mut window, LEAVING);
    let old = "old: the last answer of metrics-server had no sample of this object";
    until_hint(&mut window, &format!("{old}\nCPU "));
    assert_eq!(hints(&window, "sample of 07:59:25, 35 s ago").len(), 2, "the age of each old sample");

    // A pod that does not run has no sample: no value, and no zero.
    filter(&mut window, "gated");
    until(&mut window, "the row of gated", |window| column(window, "Name") == ["gated"]);
    assert_eq!((column(&window, "CPU"), column(&window, "Memory")), (vec!["–".to_owned()], vec!["–".to_owned()]));
}

/// Weakness 4 of plan section 6: one cluster has no metrics API, and one has lost it. Each
/// shows the state with its reason at each value, and the other cluster keeps its values.
#[test]
fn w4_a_cluster_without_the_metrics_api_shows_that_state_and_the_other_cluster_keeps_its_values() {
    let not_found = PollState::NotInstalled(error("the server could not find the requested resource (HTTP 404)"));
    let never = failed(Metrics::waiting(ClusterId::new("lon"), 15 * SECOND, now()), not_found, now() - 120 * SECOND);
    let unavailable = PollState::Unavailable(error("service unavailable (HTTP 503)"), None);
    let lost = failed(scene_metrics("fra", 2), unavailable, now() - 3 * SECOND);
    let clusters = vec![of_pods("ams", scene_metrics("ams", 1)), of_pods("fra", lost), of_pods("lon", never)];
    let mut window = wide(clusters);
    filter(&mut window, "web-6888b778b6-75md7");
    until_hint(&mut window, "CPU 460m");
    let lon = "PodMetrics: metrics-server not found: the server could not find the requested resource (HTTP 404) \
               (since 07:58:00)";
    assert_eq!(hints(&window, lon).len(), 4, "the mark and the words of each value of lon");
    let fra = "PodMetrics: metrics-server not available: service unavailable (HTTP 503) (since 07:59:57)\nCPU 920m";
    assert_eq!(hints(&window, fra).len(), 1, "the old value of fra with the reason");
    let ams = hints(&window, "CPU 460m").into_iter().filter(|hint| hint.starts_with("CPU")).count();
    assert_eq!(ams, 1, "the value of ams has no mark");
}

/// Weakness 2 of plan section 6: two clusters with a node of the same name show the value
/// of each node, against its own requests and its allocatable part.
#[test]
fn w2_two_clusters_with_a_node_of_the_same_name_show_different_values() {
    let clusters =
        vec![of_kind("ams", &nodes(), scene_metrics("ams", 1)), of_kind("fra", &nodes(), scene_metrics("fra", 3))];
    let mut window = wide(clusters);
    open_page(&mut window, "Nodes");
    let requests = "\nrequests of its pods 800m";
    until(&mut window, "the CPU of both nodes", |window| hints(window, requests).len() == 2);
    let cpu = hints(&window, requests);
    assert_ne!(cpu[0].lines().next(), cpu[1].lines().next(), "{cpu:?}");
    assert!(cpu.iter().all(|hint| hint.contains("\nallocatable 18 (")), "{cpu:?}");
    assert_eq!(column(&window, "Name"), [NODE, NODE]);
}

#[test]
fn a_workload_shows_the_sum_of_its_pods_against_their_requests_and_limits() {
    let mut window = wide(vec![of_kind("ams", &scene::<Deployment>(), scene_metrics("ams", 1))]);
    open_page(&mut window, "Deployments");
    until_hint(&mut window, "\nsum of 2 of 2 pods");
    let web = hints(&window, "\nsum of 2 of 2 pods");
    assert!(web.iter().all(|hint| hint.contains("\nrequest ") && hint.contains("\nlimit ")), "{web:?}");
    assert!(web.iter().any(|hint| hint.contains("\nrequest 200m (") && hint.contains("\nlimit 1 (")), "{web:?}");
}

/// A poll changes no store: the table asks its worker for new rows for the poll alone.
#[test]
fn a_new_poll_gives_the_rows_its_values() {
    let row = of_pods("ams", scene_metrics("ams", 1));
    let mut window = wide(vec![row.clone()]);
    filter(&mut window, "web-6888b778b6-75md7");
    until_hint(&mut window, "CPU 460m");
    let mut data = row.data.clone().expect("data");
    data.metrics = held(scene_metrics("ams", 2));
    window.state_mut().view = view(vec![ClusterRow { data: Some(data), ..row }]);
    until_hint(&mut window, "CPU 920m");
}
