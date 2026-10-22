//! The overviews of milestone M7 on the real core and two real clusters (plan section 13):
//! k3s in Docker, one with its metrics-server and one with none, and the frames of the window.
//!
//! The fake core of the view tests builds each state by hand. This test proves the way from
//! the servers to the overviews: the version of each server, the stores that the page holds
//! (`Command::ShowKinds`), the pods by phase in the namespace filter, the warning Events of
//! a pod that cannot start, and the CPU and memory of the sampler against the allocatable
//! part. The cluster with no metrics-server shows that state, and the other one keeps its
//! values (weakness 4 of plan section 6). A warning opens the panel of its pod. Last, a
//! count of the Workloads overview opens the table of the Deployments with the problems filter.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "../common/click.rs"]
mod click;
#[path = "../common/window.rs"]
mod window;

use std::time::Duration;

use eframe::egui;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_testkit::{Home, K3s, K3sOptions};
use tokio::runtime::{Builder, Runtime};

use click::click_until;
use window::{Window, core, shows, stop_with_no_ended_task, until, until_within, window};

/// The namespace of the scene.
const SHOP: &str = "shop";
/// The image of the pods: k3s has it, so the scene needs no pull.
const IMAGE: &str = "rancher/mirrored-pause:3.6";
/// The contexts of the two clusters.
const METERED: &str = "k3s-metrics";
const BARE: &str = "k3s-bare";
/// The longest wait for the first values: metrics-server gives them one or two scrapes of
/// 15 s after its start, and its image can need a pull.
const FIRST_VALUES: Duration = Duration::from_secs(300);

/// The objects of the scene, made with kubectl as a user makes them. The pod of `broken`
/// has a program that its image does not have, so it never starts and its kubelet reports
/// warnings. Its rollout fails after a deadline of 5 s, as `kubectl rollout status` tells.
const SCENE: &[&[&str]] = &[
    &["create", "namespace", SHOP],
    &["create", "deployment", "web", "--image", IMAGE, "--replicas=2", "-n", SHOP],
    &["create", "deployment", "broken", "--image", IMAGE, "-n", SHOP, "--", "/no-such-program"],
    &["patch", "deployment", "broken", "-n", SHOP, "--patch", r#"{"spec":{"progressDeadlineSeconds":5}}"#],
    &["rollout", "status", "deployment/web", "-n", SHOP, "--timeout=180s"],
];

/// Two clusters with the scene, and the app on them. The fields drop in this sequence: the
/// window and its core go before the clusters.
struct Bed {
    window: Window,
    _home: Home,
    _clusters: [K3s; 2],
    _runtime: Runtime,
}

fn bed() -> Bed {
    let runtime: Runtime = Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("a runtime");
    let metered = K3s::start_with("m7-overview-metrics", K3sOptions::default().metrics_server());
    let (metered, bare) = runtime.block_on(async { tokio::join!(metered, K3s::start("m7-overview-bare")) });
    let clusters = [metered.expect("k3s with metrics-server starts"), bare.expect("k3s starts")];
    let home = Home::new("m7-overview", &["metrics.yaml", "bare.yaml"]).expect("a home folder");
    for (k3s, (context, file)) in clusters.iter().zip([(METERED, "metrics.yaml"), (BARE, "bare.yaml")]) {
        for step in SCENE {
            runtime.block_on(k3s.kubectl(step)).unwrap_or_else(|error| panic!("kubectl {step:?}: {error}"));
        }
        let kubeconfig = k3s.kubeconfig_as(context).expect("a kubeconfig");
        std::fs::copy(kubeconfig, home.path(&format!("work/{file}"))).expect("the kubeconfig of the test cluster");
    }
    // A tall window: the blocks of both clusters show with no scroll.
    let window = window(core(&runtime, &home), egui::vec2(1600.0, 2000.0));
    Bed { window, _home: home, _clusters: clusters, _runtime: runtime }
}

fn click(window: &mut Window, role: Role, label: &str) {
    window.get_by_role_and_label(role, label).click();
    window.run();
}

/// True when the frame shows a link with this text.
fn link(window: &Window, text: &str) -> bool {
    window.query_all_by_label(text).any(|node| node.accesskit_node().role() == Role::Link)
}

/// True when the frame shows a text that contains this one.
fn shows_part(window: &Window, text: &str) -> bool {
    window.query_all_by_label_contains(text).next().is_some()
}

#[test]
#[ignore = "needs k3s in Docker"]
fn the_overviews_show_the_version_counts_usage_and_warnings_of_each_cluster_from_the_servers() {
    // The bed drops in the sequence of its fields, so no part of it moves out.
    let mut bed = bed();
    let window = &mut bed.window;
    // The first start selects the current context of the first kubeconfig file only.
    until(window, "the clusters of both files", |window| {
        window.query_by_role_and_label(Role::CheckBox, BARE).is_some()
    });
    click(window, Role::CheckBox, BARE);
    click(window, Role::RadioButton, "Cluster overview");
    until(window, "the version and the node of each cluster", |window| {
        window.query_all_by_label("v1.33.5+k3s1").count() == 2
            && window.query_all_by_label("1 of 1 node ready").count() == 2
    });

    // The namespace filter of the top bar: the pods and the Events of the scene only.
    click(window, Role::Button, "All namespaces");
    click(window, Role::CheckBox, SHOP);
    window.key_press(egui::Key::Escape);
    until(window, "the pods of the scene", |window| shows(window, "3 pods") && link(window, "1 with problems"));
    assert_eq!(window.query_all_by_label("3 pods").count(), 2, "web and broken in each cluster");
    until(window, "the warnings of broken", |window| {
        window
            .query_all_by_label_contains("Pod broken-")
            .filter(|node| node.accesskit_node().role() == Role::Link)
            .count()
            >= 2
    });

    // The sampler: values against the allocatable part in one cluster, and the state of the
    // other one, whose values do not change the first.
    let waited = until_within(window, "the CPU of the cluster with metrics-server", FIRST_VALUES, |window| {
        shows_part(window, " allocatable (") && shows_part(window, "NodeMetrics: metrics-server not found")
    });
    eprintln!("k3s overview: the first values showed after {waited:.1?}");
    assert_eq!(
        window.query_all_by_label_contains(" allocatable (").count(),
        2,
        "the CPU and the memory of one cluster"
    );
    assert!(shows_part(window, " s ago"), "the age of the sample");

    // A warning links to the panel of its object: the key of a real Event finds its pod.
    let warned = window
        .query_all_by_label_contains("Pod broken-")
        .find(|node| node.accesskit_node().role() == Role::Link)
        .expect("the link of a warning");
    let text = warned.accesskit_node().label().expect("a text");
    let pod = text.trim_start_matches("Pod ").to_owned();
    click_until(window, Role::Link, &text, |window| shows(window, "CONTAINERS") && shows(window, &pod));

    // The Workloads overview: the Deployments of each cluster by health, from the stores
    // that the page holds, and a store of the full ReplicationControllers on demand.
    click(window, Role::RadioButton, "Workloads overview");
    until(window, "the counts of the workloads", |window| {
        [METERED, BARE].iter().all(|cluster| {
            shows(window, &format!("Deployments of {cluster}: 2 objects"))
                && shows(window, &format!("ReplicationControllers of {cluster}: 0 objects"))
        })
    });
    // The broken Deployment fails at its deadline, and its count becomes a link.
    let failed = format!("Deployments of {BARE}: 1 failed");
    let link_of = |window: &Window| link(window, &failed);
    until(window, "the failed rollout of broken", link_of);
    // The broken Deployment of each cluster, of all Deployments of the two clusters.
    let rows = |window: &Window| {
        window
            .query_all_by_label_contains(" Deployments")
            .filter_map(|node| node.accesskit_node().value())
            .any(|text| text.starts_with("2 of ") && text.ends_with(" Deployments"))
    };
    click_until(window, Role::Link, &failed, |window| rows(window) && shows(window, "broken"));
    stop_with_no_ended_task(window);
}
