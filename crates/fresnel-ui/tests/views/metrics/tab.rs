//! The Metrics tab of the panel of a Pod, a workload and a Node: a chart for each resource
//! on one time axis, a tooltip with each line at the time of the pointer, a gap where
//! samples are missing, the request and the limit, and the choice of the time range.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::metrics::PollState;
use k8s_openapi::api::apps::v1::Deployment;

use super::tables::{hints, wide};
use super::{RESTART, failed, of_kind, of_pods, poll_time, scene_metrics};
use crate::data::scene;
use crate::fake::{SECOND, Window, error, now, shows, until};
use crate::kinds::scene::nodes;
use crate::workloads::scene::open;

/// Opens the Metrics tab of the panel of the object with this name on a page.
pub(super) fn metrics_tab(window: &mut Window, page: &str, name: &str) {
    open(window, page, name);
    window.get_by_role_and_label(Role::Button, "Metrics").click();
    until(window, "the charts", |window| shows(window, "CPU chart") && shows(window, "Memory chart"));
}

/// Holds the pointer over a chart at this part of its width, from its left edge, and gives
/// the lines of the tooltip: the texts that the frame did not have before.
pub(super) fn tooltip(window: &mut Window, chart: &str, part: f32) -> Vec<String> {
    let texts = |window: &Window| -> Vec<String> {
        let labels = window.query_all_by_role(Role::Label);
        labels.filter_map(|node| node.accesskit_node().value()).collect()
    };
    window.remove_cursor();
    window.run();
    let before = texts(window);
    window.ctx.all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
    let rect = window.get_by_label(chart).rect();
    window.hover_at(egui::pos2(rect.left() + rect.width() * part, rect.center().y));
    for _ in 0..4 {
        window.step();
    }
    window.run();
    texts(window).into_iter().filter(|text| !before.contains(text)).collect()
}

#[test]
fn a_pod_has_a_chart_for_each_resource_with_a_line_for_each_container_and_one_for_the_pod() {
    let mut window = wide(vec![of_pods("ams", scene_metrics("ams", 1))]);
    metrics_tab(&mut window, "Pods", "half-done");
    assert!(shows(&window, "From metrics-server, a sample each 15 s."));
    let summary = hints(&window, "CPU ").into_iter().find(|text| text.contains("  ·  ")).expect("the value of now");
    assert!(summary.ends_with("  ·  no request  ·  no limit  ·  sample of 07:59:55, 5 s ago"), "{summary}");
    let lines = tooltip(&mut window, "CPU chart", 0.98);
    let names: Vec<&str> = lines.iter().skip(1).map(|line| line.split(':').next().unwrap_or_default()).collect();
    assert_eq!(names, ["pod", "main", "worker"], "{lines:?}");
    assert!(lines[0].starts_with("CPU at 07:59:"), "{lines:?}");
    assert!(lines.iter().all(|line| !line.ends_with("no sample")), "{lines:?}");
}

/// A restart of the pod `crash` left no sample for 2 minutes: its line has a gap there, and
/// the tooltip tells it. Before and after the gap, the line has its values.
#[test]
fn a_line_has_a_gap_where_samples_are_missing() {
    let mut window = wide(vec![of_pods("ams", scene_metrics("ams", 1))]);
    metrics_tab(&mut window, "Pods", "crash");
    let since = |poll| now().duration_since(poll_time(poll)).expect("a poll before now").as_secs_f32();
    // The range is 30 minutes: a part of the width is the part of 1,800 s before now.
    let at = |seconds_ago: f32| 1.0 - seconds_ago / 1800.0;
    let middle = (since(RESTART.start) + since(RESTART.end - 1)) / 2.0;
    assert_eq!(tooltip(&mut window, "Memory chart", at(middle)).get(1).map(String::as_str), Some("crash: no sample"));
    let before = tooltip(&mut window, "Memory chart", at(since(RESTART.start) + 120.0));
    assert!(before.get(1).is_some_and(|line| line.starts_with("crash: ") && line.ends_with("Mi")), "{before:?}");
}

#[test]
fn the_range_of_the_charts_is_a_choice_and_the_charts_have_one_time_axis() {
    let mut window = wide(vec![of_pods("ams", scene_metrics("ams", 1))]);
    metrics_tab(&mut window, "Pods", "crash");
    let title = |window: &mut Window, chart| tooltip(window, chart, 0.5).into_iter().next().unwrap_or_default();
    assert!(title(&mut window, "CPU chart").starts_with("CPU at 07:4"), "the middle of 30 minutes");
    window.get_by_role_and_label(Role::Button, "5 min").click();
    window.run();
    let cpu = title(&mut window, "CPU chart");
    assert!(cpu.starts_with("CPU at 07:57:"), "the middle of 5 minutes: {cpu}");
    let memory = title(&mut window, "Memory chart");
    assert_eq!(memory.replace("Memory", "CPU"), cpu, "one time axis");
}

#[test]
fn a_workload_has_the_line_of_its_pods_and_of_each_pod_with_the_request_and_the_limit() {
    let mut window = wide(vec![of_kind("ams", &scene::<Deployment>(), scene_metrics("ams", 1))]);
    metrics_tab(&mut window, "Deployments", "web");
    let lines = tooltip(&mut window, "CPU chart", 0.98);
    let names: Vec<&str> = lines.iter().skip(1).map(|line| line.split(':').next().unwrap_or_default()).collect();
    assert_eq!(names, ["all pods", "web-6888b778b6-75md7", "web-6888b778b6-p2twl", "request", "limit"], "{lines:?}");
    assert!(lines.contains(&"request: 200m".to_owned()) && lines.contains(&"limit: 1".to_owned()), "{lines:?}");
}

/// The names of the lines and marks of the tooltip of the CPU chart near now.
pub(super) fn names(window: &mut Window) -> Vec<String> {
    let lines = tooltip(window, "CPU chart", 0.98);
    lines.iter().skip(1).map(|line| line.split(':').next().unwrap_or_default().to_owned()).collect()
}

#[test]
fn the_legend_names_each_pod_by_the_end_of_its_name_and_a_click_hides_its_line_and_shows_it_again() {
    let mut window = wide(vec![of_kind("ams", &scene::<Deployment>(), scene_metrics("ams", 1))]);
    metrics_tab(&mut window, "Deployments", "web");
    for name in ["all pods", "75md7", "p2twl"] {
        let entries = window.query_all_by_role_and_label(Role::Button, name).count();
        assert_eq!(entries, 2, "{name} in the legend of each chart");
    }
    let click = |window: &mut Window| {
        window.query_all_by_role_and_label(Role::Button, "75md7").next().expect("the legend of CPU").click();
        window.run();
    };
    click(&mut window);
    assert_eq!(names(&mut window), ["all pods", "web-6888b778b6-p2twl", "request", "limit"]);
    click(&mut window);
    assert_eq!(names(&mut window), ["all pods", "web-6888b778b6-75md7", "web-6888b778b6-p2twl", "request", "limit"]);
}

#[test]
fn a_node_has_its_usage_with_the_requests_of_its_pods_its_allocatable_part_and_its_capacity() {
    let mut window = wide(vec![of_kind("ams", &nodes(), scene_metrics("ams", 1))]);
    metrics_tab(&mut window, "Nodes", super::NODE);
    let lines = tooltip(&mut window, "Memory chart", 0.98);
    let names: Vec<&str> = lines.iter().skip(1).map(|line| line.split(':').next().unwrap_or_default()).collect();
    assert_eq!(names, ["node", "requests of its pods", "allocatable", "capacity"], "{lines:?}");
    assert!(lines.contains(&"requests of its pods: 710Mi".to_owned()), "{lines:?}");
}

#[test]
fn an_object_with_no_sample_says_so() {
    let mut window = wide(vec![of_pods("ams", scene_metrics("ams", 1))]);
    metrics_tab(&mut window, "Pods", "gated");
    assert!(shows(&window, "The metrics have no sample of this object yet."));
}

/// Weakness 4 of plan section 6 in the panel: a list that fails keeps the history in the
/// charts, and the value of now tells the reason first.
#[test]
fn the_charts_of_a_cluster_whose_list_fails_keep_the_history_and_the_value_tells_why_it_is_old() {
    let unavailable = PollState::Unavailable(error("service unavailable (HTTP 503)"), None);
    let metrics = failed(scene_metrics("fra", 1), unavailable, now() - 3 * SECOND);
    let mut window = wide(vec![of_pods("fra", metrics)]);
    metrics_tab(&mut window, "Pods", "crash");
    let reason = "PodMetrics: metrics-server not available: service unavailable (HTTP 503) (since 07:59:57)";
    let summaries = hints(&window, &format!("{reason}  ·  "));
    assert_eq!(summaries.len(), 2, "the value of each resource: {summaries:?}");
    let lines = tooltip(&mut window, "CPU chart", 0.5);
    assert!(lines.get(1).is_some_and(|line| line.starts_with("crash: ") && line.ends_with('m')), "{lines:?}");
}
