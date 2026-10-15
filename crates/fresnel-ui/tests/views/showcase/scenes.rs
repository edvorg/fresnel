//! The pictures of the pages: the Pods of three clusters, the quick search, a drain in the
//! Operations tab and the cluster overview in the light theme. Each one shows a state that
//! the app reaches with the data of the world.

use std::sync::Arc;

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::JobView;
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::ops::drain::{Drain, DrainOptions, DrainPod, PodState, Verdict};
use fresnel_core::ops::{JobState, Operation, Outcome, StepState, Target};
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::core::v1::Pod;

use super::objects::{suffix, uid};
use super::world::DRAINED;
use super::{dock_edge, window};
use crate::fake::{Window, open_page, open_tab, until};
use crate::pods::resize::drag;
use crate::snapshots::save_clean;
use crate::workloads::scene::{job, jobs};

/// The pods of the three clusters with their CPU and memory; the rows of prod-us are old.
/// The Namespace and Cluster columns are narrower, so that Memory fits.
#[test]
fn the_pods_of_three_clusters() {
    let mut window = window(egui::Theme::Dark);
    drag(&mut window, "Namespace", -25.0);
    drag(&mut window, "Cluster", -45.0);
    until(&mut window, "the metrics", |window| window.query_all_by_label_contains("\nsample of ").next().is_some());
    save_clean(&mut window, "pods-three-clusters");
}

/// The quick search for "checkout": the objects of each kind in each cluster.
#[test]
fn the_quick_search() {
    let mut window = window(egui::Theme::Dark);
    window.key_press_modifiers(Modifiers::COMMAND, Key::K);
    window.run();
    window.get_by_label("Search text").type_text("checkout");
    until(&mut window, "the hits", |window| {
        window.query_by_label("Deployment shop/checkout-api of staging").is_some()
            && window.query_by_label("updating…").is_none()
    });
    save_clean(&mut window, "quick-search");
}

/// An object of prod-eu as the target of an operation.
fn target(kind: Kind, namespace: &str, name: &str) -> Target {
    let uid = uid(&["prod-eu", kind.info().name, namespace, name]);
    Target { cluster: ClusterId::new("prod-eu"), kind, key: ObjectKey::new(namespace, name), uid: uid.into() }
}

/// The drain of the node of prod-eu: a pod of a DaemonSet stays, three pods are gone, and
/// the disruption budget of postgres holds postgres-1.
fn drain(window: &Window) -> Arc<JobView> {
    // The pod of an app on the node of the drain.
    let on_node = |app: &str| {
        let on = |pod: &Pod| {
            pod.spec.as_ref().and_then(|spec| spec.node_name.as_deref()) == Some(DRAINED)
                && pod.metadata.name.as_deref().is_some_and(|name| name.starts_with(app))
        };
        super::pods_of_prod_eu(window, on).pop().expect("a pod on the node")
    };
    let evicted = |namespace: &str, set: &str| {
        let name = format!("{set}-{}", suffix(&[set, "evicted"], 5));
        DrainPod { key: ObjectKey::new(namespace, &name), verdict: Verdict::Evict(None), state: PodState::Gone }
    };
    let budget = "The disruption budget postgres needs 1 healthy pods and has 1 currently";
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
    let steps = [("cordon the node", StepState::Done("the node takes no new pod".to_owned()))];
    let mut drain = job(4, target(Kind::Node, "", DRAINED), Operation::Drain(options), JobState::Pending, &steps);
    Arc::make_mut(&mut drain).report.outcome =
        Outcome::Drain(Drain { cordoned: true, written: None, pods: pods.into() });
    drain
}

/// The Nodes of the three clusters and the Operations tab with a drain that a disruption
/// budget holds, after three jobs that are done.
#[test]
fn a_node_drain_in_the_operations_tab() {
    let mut window = window(egui::Theme::Dark);
    open_page(&mut window, "Nodes");
    let done = |number, kind, (namespace, name), operation| {
        job(number, target(kind, namespace, name), operation, JobState::Done, &[])
    };
    let mut core = window.state().view.clone();
    core.jobs = jobs(
        vec![
            done(1, Kind::Deployment, ("shop", "cart"), Operation::Scale(3)),
            done(2, Kind::Deployment, ("payments", "payments-worker"), Operation::Restart),
            done(3, Kind::Deployment, ("shop", "checkout-api"), Operation::History),
            drain(&window),
        ],
        0,
    );
    window.state_mut().view = core;
    open_tab(&mut window, "Operations");
    // The dock grows to show the pods of the drain.
    let edge = dock_edge(&window);
    super::move_edge(&mut window, edge, edge - egui::vec2(0.0, 200.0));
    until(&mut window, "the metrics", |window| window.query_all_by_label_contains("\nallocatable ").next().is_some());
    save_clean(&mut window, "node-drain");
}

/// The cluster overview of the three clusters in the light theme.
#[test]
fn the_cluster_overview_in_the_light_theme() {
    let mut window = window(egui::Theme::Light);
    crate::overview::open(&mut window, "Cluster overview");
    save_clean(&mut window, "cluster-overview-light");
}
