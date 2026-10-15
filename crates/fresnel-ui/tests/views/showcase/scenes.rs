//! The pictures of the pages: the Pods of three clusters, the quick search, a drain in the
//! Operations tab and the cluster overview in the light theme. Each one shows a state that
//! the app reaches with the data of the world.

use std::sync::Arc;

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::JobView;
use fresnel_core::kinds::Kind;
use fresnel_core::ops::drain::{Drain, DrainOptions, DrainPod, PodState, Verdict};
use fresnel_core::ops::{JobState, Operation, Outcome, StepState};
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::core::v1::Pod;

use super::objects::suffix;
use super::world::{DRAINED, PROD_EU, PROD_US};
use super::{MARGIN, TO_THE_END, save, target, window};
use crate::fake::{JOB_WIDENING, Window, assert_dragged, grow_dock, move_edge, open_page, open_tab, until, widen_job};
use crate::frame_time::wheel;
use crate::pods::resize::drag;
use crate::workloads::scene::{job, jobs};

/// Makes the Namespace and Cluster columns of the Pods narrower, so that Memory fits.
fn narrow_pod_columns(window: &mut Window) {
    drag(window, "Namespace", -25.0);
    drag(window, "Cluster", -45.0);
}

/// The pods of the three clusters with their CPU and memory; the rows of prod-us are old.
#[test]
fn the_pods_of_three_clusters() {
    let mut window = window(egui::Theme::Dark);
    narrow_pod_columns(&mut window);
    until(&mut window, "the metrics", |window| window.query_all_by_label_contains("\nsample of ").next().is_some());
    save(&mut window, "pods-three-clusters");
}

/// The quick search for "checkout": the objects of each kind in each cluster.
#[test]
fn the_quick_search() {
    let mut window = window(egui::Theme::Dark);
    narrow_pod_columns(&mut window);
    window.key_press_modifiers(Modifiers::COMMAND, Key::K);
    window.run();
    window.get_by_label("Search text").type_text("checkout");
    until(&mut window, "the hits", |window| {
        window.query_by_label("Deployment shop/checkout-api of staging").is_some()
            && window.query_by_label("updating…").is_none()
    });
    save(&mut window, "quick-search");
}

/// The drain of the node of prod-eu, at its evictions: a pod of a DaemonSet stays, three pods
/// are gone, and the disruption budget of postgres holds postgres-1. The steps have the texts
/// of the drain of the core.
fn drain(window: &Window) -> Arc<JobView> {
    // The pod of an app on the node of the drain.
    let on_node = |app: &str| {
        let on = |pod: &Pod| {
            pod.spec.as_ref().and_then(|spec| spec.node_name.as_deref()) == Some(DRAINED)
                && pod.metadata.name.as_deref().is_some_and(|name| name.starts_with(app))
        };
        super::pods_of(window, PROD_EU.name, on).pop().expect("a pod on the node")
    };
    let evicted = |namespace: &str, set: &str| {
        let name = format!("{set}-{}", suffix(&[set, "evicted"], 5));
        DrainPod { key: ObjectKey::new(namespace, &name), verdict: Verdict::Evict(None), state: PodState::Gone }
    };
    let budget = "The disruption budget postgres needs 2 healthy pods and has 2 currently";
    let pods = [
        DrainPod {
            key: ObjectKey::new("monitoring", &on_node("node-exporter")),
            verdict: Verdict::Keep("a pod of a DaemonSet"),
            state: PodState::Kept,
        },
        evicted("shop", "cart-5c8f7b9d6"),
        evicted("monitoring", "grafana-7f5b6c8d4"),
        evicted("kube-system", "coredns-5d78c9869d"),
        DrainPod {
            key: ObjectKey::new("payments", &on_node("postgres")),
            verdict: Verdict::Evict(None),
            state: PodState::Blocked { budget: Some("postgres".to_owned()), reason: budget.to_owned(), attempts: 9 },
        },
    ];
    let options = DrainOptions { ignore_daemon_sets: true, ..DrainOptions::default() };
    let done = |text: &str| StepState::Done(text.to_owned());
    let steps = [
        ("cordon the node", done("the node takes no new pod")),
        ("list the pods on the node", done("5 pods")),
        ("read the DaemonSets of the pods: 1", done("read")),
        ("check each pod", done("4 to evict, 1 to keep")),
        ("evict 4 pods and wait until each one is gone", StepState::Running),
    ];
    let node = target(PROD_EU.name, Kind::Node, "", DRAINED);
    let mut drain = job(4, node, Operation::Drain(options), JobState::Pending, &steps);
    Arc::make_mut(&mut drain).report.outcome =
        Outcome::Drain(Drain { cordoned: true, written: None, pods: pods.into() });
    drain
}

/// The dock grows this much to show each step and each pod of the drain.
const DRAIN_DOCK: f32 = 280.0;
/// The Status column of the Nodes gets this much wider, so "Ready,SchedulingDisabled" fits.
/// The Name column, wider than the names of the nodes, gives that room, and the Roles and
/// External IP columns, with no value, give the room of Memory.
const STATUS_WIDENING: f32 = 60.0;
const NAME_NARROWING: f32 = 90.0;
const ROLES_NARROWING: f32 = 85.0;
const EXTERNAL_NARROWING: f32 = 40.0;

/// Drags the edge at the right of a column title of the Nodes `by` points to the left. The
/// drag of the Pod tests knows only the titles of the Pods.
fn narrow_column(window: &mut Window, title: &str, by: f32) {
    let width = |window: &Window| window.get_by_role_and_label(Role::Button, title).rect().width();
    let (before, header) = (width(window), window.get_by_role_and_label(Role::Button, title).rect());
    let edge = header.right_center();
    move_edge(window, edge, edge - egui::vec2(by, 0.0));
    assert_dragged(window, title, before - width(window), by);
}

/// The Nodes of the three clusters and the Operations tab with a drain that a disruption
/// budget holds, after three jobs that are done.
#[test]
fn a_node_drain_in_the_operations_tab() {
    let mut window = window(egui::Theme::Dark);
    open_page(&mut window, "Nodes");
    drag(&mut window, "Name", -NAME_NARROWING);
    drag(&mut window, "Status", STATUS_WIDENING);
    narrow_column(&mut window, "Roles", ROLES_NARROWING);
    narrow_column(&mut window, "External IP", EXTERNAL_NARROWING);
    let memory = window.get_by_role_and_label(Role::Button, "Memory").rect();
    assert!(memory.right() <= window.ctx.content_rect().right() - MARGIN, "Memory fits: {memory:?}");
    let done = |number, kind, (namespace, name), operation| {
        job(number, target(PROD_EU.name, kind, namespace, name), operation, JobState::Done, &[])
    };
    let mut core = window.state().view.clone();
    core.jobs = jobs(
        vec![
            done(1, Kind::Deployment, ("shop", "cart"), Operation::History),
            done(2, Kind::Deployment, ("shop", "cart"), Operation::Scale(3)),
            done(3, Kind::StatefulSet, ("shop", "redis"), Operation::Restart),
            drain(&window),
        ],
        0,
    );
    window.state_mut().view = core;
    open_tab(&mut window, "Operations");
    grow_dock(&mut window, DRAIN_DOCK);
    // The verdicts fit beside the names of the pods, and the job list keeps its room.
    widen_job(&mut window, ("Job 4: Drain", "postgres-1"), JOB_WIDENING);
    until(&mut window, "the metrics", |window| window.query_all_by_label_contains("\nallocatable ").next().is_some());
    save(&mut window, "node-drain");
}

/// The top of the card of a cluster in the Connections tab: its line with the server.
fn card_top(window: &Window, cluster: &str) -> f32 {
    window.get_by_label_contains(&format!("{cluster}.k8s.example")).rect().top()
}

/// The reason in the head of the card of prod-us of the overview.
const PROD_US_REASON: &str = "watches: connect: no answer in 3s";

/// Scrolls the page to its end: the middle of the window is on it.
fn to_the_end(window: &mut Window) {
    window.hover_at(window.ctx.content_rect().center());
    window.event(wheel(TO_THE_END));
    window.run();
}

/// The cluster overview of the three clusters in the light theme, at its end with the
/// release that crashes on staging, and the connections of prod-us in the dock: each
/// channel tries again after a pause. The dock takes the room above the card of prod-us,
/// so the page starts with that card and the dock shows each channel.
#[test]
fn the_cluster_overview_in_the_light_theme() {
    let mut window = window(egui::Theme::Light);
    crate::overview::open(&mut window, "Cluster overview");
    let page_top = window.get_by_role_and_label(Role::Label, "Cluster overview").rect().top();
    let card = |window: &Window| window.get_by_label(PROD_US_REASON).rect().top();
    to_the_end(&mut window);
    let rise = card(&window) - page_top;
    grow_dock(&mut window, rise);
    to_the_end(&mut window);
    assert!((card(&window) - page_top).abs() <= 1.0, "the card of prod-us is at the top of the page");
    open_tab(&mut window, "Connections");
    // The list moves up by the card of prod-eu, so the card of prod-us is at its top.
    let first = card_top(&window, PROD_EU.name);
    let card = card_top(&window, PROD_US.name) - first;
    window.hover_at(window.get_by_label_contains("prod-eu.k8s.example").rect().center());
    window.event(wheel(-card));
    window.run();
    assert!((card_top(&window, PROD_US.name) - first).abs() <= 1.0, "the card of prod-us is at the top");
    save(&mut window, "cluster-overview-light");
}
