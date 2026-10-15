//! The metrics of milestone M6 on the real core and a real cluster (plan section 13): k3s
//! in Docker with its metrics-server, the sampler of the core, and the frames of the window.
//!
//! The fake core of the view tests builds the metrics by hand. This test proves the way from
//! metrics-server to a view: the CPU and memory of a pod with the request and the limit of
//! its spec, the sum of a Deployment against the requests and limits of its pods, a node
//! against the requests of its pods and its allocatable part, a line of the history of the
//! sampler in a chart of the Metrics tab, and the sampler in the Connections view.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "../common/window.rs"]
mod window;

use std::time::Duration;

use eframe::egui;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_testkit::{Home, K3s, K3sOptions};
use tokio::runtime::{Builder, Runtime};

use window::{Window, core, shows, stop_with_no_ended_task, until, until_within, window};

/// The namespace of the scene.
const SHOP: &str = "shop";
/// The image of the pods: k3s has it, so the scene needs no pull.
const IMAGE: &str = "rancher/mirrored-pause:3.6";
/// The longest wait for the first values: metrics-server gives them one or two scrapes of
/// 15 s after its start, and its image can need a pull.
const FIRST_VALUES: Duration = Duration::from_secs(300);

/// The objects of the scene, made with kubectl as a user makes them. The Deployment gets its
/// resources before its first pod, so no pod of an older ReplicaSet ends during the test.
const SCENE: &[&[&str]] = &[
    &["create", "namespace", SHOP],
    &["create", "deployment", "web", "--image", IMAGE, "--replicas=0", "-n", SHOP],
    &[
        "set",
        "resources",
        "deployment/web",
        "--requests=cpu=50m,memory=32Mi",
        "--limits=cpu=200m,memory=64Mi",
        "-n",
        SHOP,
    ],
    &["scale", "deployment/web", "--replicas=2", "-n", SHOP],
    &["rollout", "status", "deployment/web", "-n", SHOP, "--timeout=180s"],
];

/// A cluster with the scene, and the app on it. The fields drop in this sequence: the
/// window and its core go before the cluster.
struct Bed {
    window: Window,
    _home: Home,
    k3s: K3s,
    runtime: Runtime,
}

fn bed() -> Bed {
    let runtime: Runtime = Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("a runtime");
    let k3s = runtime.block_on(K3s::start_with("m6-views", K3sOptions::default().metrics_server()));
    let k3s = k3s.expect("k3s starts");
    for step in SCENE {
        runtime.block_on(k3s.kubectl(step)).unwrap_or_else(|error| panic!("kubectl {step:?}: {error}"));
    }
    let home = Home::new("m6-views", &["kubeconfig.yaml"]).expect("a home folder");
    std::fs::copy(k3s.kubeconfig(), home.path("work/kubeconfig.yaml")).expect("the kubeconfig of the test cluster");
    // A wide window: each column of the Pods, and the panel with its charts.
    let window = window(core(&runtime, &home), egui::vec2(2000.0, 1400.0));
    Bed { window, _home: home, k3s, runtime }
}

/// Opens the page of a kind with the rows that have this text.
fn page(window: &mut Window, page: &str, text: &str) {
    window.get_by_role_and_label(Role::RadioButton, page).click();
    window.run();
    window.get_by_role(Role::TextInput).click();
    window.run();
    window.get_by_role(Role::TextInput).type_text(text);
    window.run();
}

/// The words of the cells and labels that have this text.
fn hints(window: &Window, text: &str) -> Vec<String> {
    window.query_all_by_label_contains(text).filter_map(|node| node.accesskit_node().value()).collect()
}

/// The lines of the tooltip of a chart with the pointer at this part of its width: the
/// texts that the frame did not have before.
fn tooltip(window: &mut Window, chart: &str, part: f32) -> Vec<String> {
    let texts = |window: &Window| -> Vec<String> {
        window.query_all_by_role(Role::Label).filter_map(|node| node.accesskit_node().value()).collect()
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

/// The tooltip of the CPU chart at the first place, from now back, where the line of the
/// container has a value: the samples of the sampler are in the line. Its first samples
/// can be few, so the pointer moves over the range of 5 minutes in steps of 3 s.
fn tooltip_with_a_value(window: &mut Window, container: &str) -> Vec<String> {
    window.get_by_role_and_label(Role::Button, "5 min").click();
    window.run();
    let mut seen = Vec::new();
    for step in 1..100u8 {
        let lines = tooltip(window, "CPU chart", 1.0 - f32::from(step) / 100.0);
        let valued = lines.iter().any(|line| line.starts_with(&format!("{container}: ")) && line.ends_with('m'));
        if valued {
            return lines;
        }
        seen = lines;
    }
    panic!("no value of {container} in the CPU chart; the last tooltip: {seen:?}");
}

#[test]
#[ignore = "needs k3s in Docker"]
fn the_metrics_of_metrics_server_show_in_the_tables_the_charts_and_the_connections_view() {
    // The bed drops in the sequence of its fields, so no part of it moves out.
    let mut bed = bed();
    let (window, k3s, runtime) = (&mut bed.window, &bed.k3s, &bed.runtime);
    let pods = runtime.block_on(k3s.kubectl(&["get", "pods", "-n", SHOP, "-o", "jsonpath={.items[*].metadata.name}"]));
    let pods: Vec<String> = pods.expect("the pods").split_whitespace().map(str::to_owned).collect();
    assert_eq!(pods.len(), 2, "the pods of the Deployment");
    until(window, "the stores of the cluster", |window| shows(window, "Deployments"));

    // Each pod: its usage against the request and the limit of its spec, with the age.
    page(window, "Pods", "web-");
    let request = "\nrequest 50m (";
    let waited = until_within(window, "the CPU of both pods", FIRST_VALUES, |window| hints(window, request).len() == 2);
    eprintln!("k3s metrics: the first values showed after {waited:.1?}");
    let cpu = hints(window, request);
    assert!(cpu.iter().all(|hint| hint.starts_with("CPU ") && hint.contains("\nlimit 200m (")), "{cpu:?}");
    assert!(cpu.iter().all(|hint| hint.contains(" s ago")), "the age of each sample: {cpu:?}");
    assert_eq!(hints(window, "\nrequest 32Mi (").len(), 2, "the memory of both pods");

    // The Deployment: the sum of its two pods against the sums of their spec.
    page(window, "Deployments", "web");
    until(window, "the sum of web", |window| !hints(window, "\nsum of 2 of 2 pods").is_empty());
    let web = hints(window, "\nsum of 2 of 2 pods");
    assert!(web.iter().any(|hint| hint.contains("\nrequest 100m (") && hint.contains("\nlimit 400m (")), "{web:?}");

    // The charts of a pod, from the history of the sampler.
    page(window, "Pods", &pods[0]);
    until(window, "the row of the pod", |window| hints(window, request).len() == 1);
    window.query_all_by_label(&pods[0]).next().expect("the row").click();
    until(window, "the panel of the pod", |window| shows(window, "Metrics"));
    window.get_by_role_and_label(Role::Button, "Metrics").click();
    until(window, "the charts", |window| shows(window, "CPU chart") && shows(window, "Memory chart"));
    // The period is the window of metrics-server when it is longer than 15 s.
    let source = hints(window, "From metrics-server, a sample each ");
    eprintln!("k3s metrics: {source:?}");
    assert_eq!(source.len(), 1, "the source of the values");
    let container = runtime.block_on(k3s.kubectl(&[
        "get",
        "deployment/web",
        "-n",
        SHOP,
        "-o",
        "jsonpath={.spec.template.spec.containers[0].name}",
    ]));
    let lines = tooltip_with_a_value(window, container.expect("the container").trim());
    let marks = ["request: 50m", "limit: 200m"].map(str::to_owned);
    assert!(marks.iter().all(|mark| lines.contains(mark)), "the request and the limit of the spec: {lines:?}");

    // The node: its usage against the requests of its pods and its allocatable part.
    let node = runtime.block_on(k3s.kubectl(&["get", "nodes", "-o", "jsonpath={.items[0].metadata.name}"]));
    page(window, "Nodes", node.expect("the node").trim());
    until(window, "the CPU of the node", |window| !hints(window, "\nallocatable ").is_empty());
    let requests = hints(window, "% of allocatable)\nallocatable ");
    assert!(requests.iter().any(|hint| hint.contains("\nrequests of its pods ")), "the node: {requests:?}");

    // The sampler: both lists answer.
    window.get_by_role_and_label(Role::RadioButton, "Connections").click();
    until(window, "the sampler", |window| shows(window, "PodMetrics") && shows(window, "NodeMetrics"));
    assert_eq!(hints(window, "ok, last answer ").len(), 2, "both lists of the sampler answer");
    stop_with_no_ended_task(window);
}
