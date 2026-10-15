//! PNG files of the metrics in the dark theme, for a look by a person: the CPU and memory
//! columns of the Pods with an old value and a cluster without the metrics API, the Nodes
//! of two clusters, the Metrics tab of a pod with its tooltip, of a workload and of a node,
//! and the sampler of each cluster in the Connections view.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::cluster::ClusterId;
use fresnel_core::metrics::{Metrics, PollState};
use k8s_openapi::api::apps::v1::Deployment;

use super::{failed, of_kind, of_pods, scene_metrics};
use crate::data::scene;
use crate::fake::{SECOND, error, now, open_page, shows, until, view, window_with};
use crate::kinds::scene::nodes;
use crate::snapshots::{SCALE, panel_of, save, save_clean};

/// `ams` with metrics, `fra` whose PodMetrics fail since a moment, and `lon` with no
/// metrics API.
fn three_clusters() -> Vec<fresnel_core::app::ClusterRow> {
    let unavailable = PollState::Unavailable(error("service unavailable (HTTP 503)"));
    let not_found = PollState::NotInstalled(error("the server could not find the requested resource (HTTP 404)"));
    let never = Metrics::waiting(ClusterId::new("lon"), 15 * SECOND, now());
    vec![
        of_pods("ams", scene_metrics("ams", 1)),
        of_pods("fra", failed(scene_metrics("fra", 2), unavailable, now() - 40 * SECOND)),
        of_pods("lon", failed(never, not_found, now() - 300 * SECOND)),
    ]
}

#[test]
fn the_pods_with_cpu_and_memory() {
    let mut window = window_with(view(vec![of_pods("ams", scene_metrics("ams", 1))]), egui::Theme::Dark, SCALE);
    window.set_size(egui::vec2(1700.0, 800.0));
    window.run();
    until(&mut window, "the values", |window| window.query_all_by_label_contains("\nsample of ").next().is_some());
    save_clean(&mut window, "m6-pods");

    let mut window = window_with(view(three_clusters()), egui::Theme::Dark, SCALE);
    window.set_size(egui::vec2(1700.0, 800.0));
    window.run();
    window.get_by_role(Role::TextInput).click();
    window.get_by_role(Role::TextInput).type_text("web-");
    until(&mut window, "the values", |window| window.query_by_label_contains("CPU 460m").is_some());
    save_clean(&mut window, "m6-pods-three-clusters");
}

#[test]
fn the_nodes_of_two_clusters_with_a_node_of_the_same_name() {
    let clusters =
        vec![of_kind("ams", &nodes(), scene_metrics("ams", 1)), of_kind("fra", &nodes(), scene_metrics("fra", 3))];
    let mut window = window_with(view(clusters), egui::Theme::Dark, SCALE);
    window.set_size(egui::vec2(1700.0, 600.0));
    window.run();
    open_page(&mut window, "Nodes");
    until(&mut window, "the values", |window| window.query_all_by_label_contains("\nallocatable 18").count() == 2);
    save_clean(&mut window, "m6-nodes");
}

#[test]
fn the_metrics_tab_of_a_pod_with_its_tooltip_of_a_workload_and_of_a_node() {
    let mut window = panel_of(vec![of_pods("ams", scene_metrics("ams", 1))], "Pods", "half-done");
    window.set_size(egui::vec2(crate::fake::WINDOW.x, 1000.0));
    window.get_by_role_and_label(Role::Button, "Metrics").click();
    until(&mut window, "the charts", |window| shows(window, "Memory chart"));
    save_clean(&mut window, "m6-pod-metrics");
    window.ctx.all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
    let rect = window.get_by_label("CPU chart").rect();
    window.hover_at(egui::pos2(rect.left() + rect.width() * 0.8, rect.center().y));
    for _ in 0..4 {
        window.step();
    }
    window.run();
    save(&mut window, "m6-pod-metrics-tooltip");

    let deployments = vec![of_kind("ams", &scene::<Deployment>(), scene_metrics("ams", 1))];
    let mut window = panel_of(deployments, "Deployments", "web");
    window.set_size(egui::vec2(crate::fake::WINDOW.x, 1000.0));
    window.get_by_role_and_label(Role::Button, "Metrics").click();
    until(&mut window, "the charts", |window| shows(window, "Memory chart"));
    save_clean(&mut window, "m6-deployment-metrics");

    let mut window = panel_of(vec![of_kind("ams", &nodes(), scene_metrics("ams", 1))], "Nodes", super::NODE);
    window.set_size(egui::vec2(crate::fake::WINDOW.x, 1000.0));
    window.get_by_role_and_label(Role::Button, "Metrics").click();
    until(&mut window, "the charts", |window| shows(window, "Memory chart"));
    save_clean(&mut window, "m6-node-metrics");
}

#[test]
fn the_sampler_of_each_cluster_in_the_connections_view() {
    let mut window = window_with(view(three_clusters()), egui::Theme::Dark, SCALE);
    open_page(&mut window, "Connections");
    until(&mut window, "the samplers", |window| shows(window, "NodeMetrics"));
    save_clean(&mut window, "m6-connections");
}
