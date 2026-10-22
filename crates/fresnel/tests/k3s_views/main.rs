//! The detail views of milestone M7 on the real core and a real cluster (plan section 13):
//! k3s in Docker, the stores and the indexes of the core, and the frames of the window.
//!
//! The fake core of the view tests builds each state by hand. This test proves the way from
//! the server to a panel: the endpoints of a Service from the owner index, the quotas and
//! the counts of a Namespace from the stores that its page holds, the bindings of a
//! ServiceAccount from the stores of the full bindings, and the pods on a Node. Last, a
//! click drains the node: the dialog, the job of the core and the Operations tab.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "../common/click.rs"]
mod click;
#[path = "../common/window.rs"]
mod window;

use eframe::egui;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_testkit::{Home, K3s};
use tokio::runtime::{Builder, Runtime};

use click::click_until;
use window::{Window, core, shows, stop_with_no_ended_task, until, window};

/// The namespace of the scene.
const SHOP: &str = "shop";
/// The image of the pods: k3s has it, so the scene needs no pull.
const IMAGE: &str = "rancher/mirrored-pause:3.6";

/// The objects of the scene, made with kubectl as a user makes them.
const SCENE: &[&[&str]] = &[
    &["create", "namespace", SHOP],
    &["create", "quota", "quota", "--hard=pods=10,services=5", "-n", SHOP],
    &["create", "deployment", "web", "--image", IMAGE, "--replicas=2", "-n", SHOP],
    &["expose", "deployment", "web", "--port=80", "--target-port=8080", "-n", SHOP],
    &["create", "serviceaccount", "robot", "-n", SHOP],
    &["create", "role", "reader", "--verb=get,list", "--resource=pods", "-n", SHOP],
    &["create", "rolebinding", "reader", "--role=reader", "--serviceaccount=shop:robot", "--user=alice", "-n", SHOP],
    &["create", "clusterrolebinding", "shop-viewer", "--clusterrole=view", "--serviceaccount=shop:robot"],
    &["rollout", "status", "deployment/web", "-n", SHOP, "--timeout=180s"],
    &["wait", "--for=jsonpath={.status.used.pods}=2", "resourcequota/quota", "-n", SHOP, "--timeout=60s"],
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
    let k3s = runtime.block_on(K3s::start("m7-views")).expect("k3s starts");
    for step in SCENE {
        runtime.block_on(k3s.kubectl(step)).unwrap_or_else(|error| panic!("kubectl {step:?}: {error}"));
    }
    let home = Home::new("m7-views", &["kubeconfig.yaml"]).expect("a home folder");
    std::fs::copy(k3s.kubeconfig(), home.path("work/kubeconfig.yaml")).expect("the kubeconfig of the test cluster");
    // A tall window: the sidebar and the panel show each entry with no scroll.
    let window = window(core(&runtime, &home), egui::vec2(1400.0, 1800.0));
    Bed { window, _home: home, k3s, runtime }
}

/// Opens the page of a kind and the panel of the object of the row with this name.
fn open(window: &mut Window, page: &str, name: &str) {
    window.get_by_role_and_label(Role::RadioButton, page).click();
    window.run();
    window.get_by_role(Role::TextInput).click();
    window.run();
    window.get_by_role(Role::TextInput).type_text(name);
    click_until(window, Role::Label, name, |window| shows(window, "Overview"));
}

/// Clicks the button with this label.
fn click(window: &mut Window, label: &str) {
    window.get_by_role_and_label(Role::Button, label).click();
    window.run();
}

/// Clicks the link of the panel with this text.
fn follow(window: &mut Window, text: &str) {
    let link = window.query_all_by_label(text).find(|node| node.accesskit_node().role() == Role::Link);
    link.unwrap_or_else(|| panic!("the link {text}")).click_accesskit();
    window.run();
}

#[test]
#[ignore = "needs k3s in Docker"]
fn the_views_of_a_service_a_namespace_a_binding_an_account_and_a_node_show_the_objects_of_the_server() {
    // The bed drops in the sequence of its fields, so no part of it moves out.
    let mut bed = bed();
    let (window, k3s, runtime) = (&mut bed.window, &bed.k3s, &bed.runtime);
    let pods = runtime.block_on(k3s.kubectl(&["get", "pods", "-n", SHOP, "-o", "jsonpath={.items[*].metadata.name}"]));
    let pods: Vec<String> = pods.expect("the pods").split_whitespace().map(str::to_owned).collect();
    assert_eq!(pods.len(), 2, "the pods of the Deployment");
    until(window, "the stores of the cluster", |window| shows(window, "Services"));

    // The endpoints come from the owner index, with the pod of each one as a link.
    open(window, "Services", "web");
    until(window, "the endpoints of web", |window| pods.iter().all(|pod| shows(window, pod)));
    assert!(shows(window, "80/TCP") && shows(window, "8080") && shows(window, "ready"));

    // The quota comes from the store of the full quotas that the page of the Namespaces
    // holds, and the counts from the stores that are always on.
    open(window, "Namespaces", SHOP);
    until(window, "the quota and the counts of shop", |window| {
        shows(window, "ResourceQuota quota") && shows(window, "20%") && shows(window, "Deployments")
    });

    // The bindings of the account come from the stores of the full bindings.
    open(window, "RoleBindings", "reader");
    until(window, "the subjects of reader", |window| shows(window, "alice") && shows(window, "Role reader"));
    follow(window, "robot");
    until(window, "the bindings of robot", |window| {
        shows(window, "RoleBinding reader") && shows(window, "ClusterRoleBinding shop-viewer")
    });

    // The pods on the node come from the reference index.
    let node = runtime.block_on(k3s.kubectl(&["get", "nodes", "-o", "jsonpath={.items[0].metadata.name}"]));
    let node = node.expect("the node");
    open(window, "Nodes", node.trim());
    until(window, "the pods on the node", |window| shows(window, "CAPACITY") && shows(window, &pods[0]));

    // The dialog reads the verdicts from the stores, the core runs the drain, and the
    // Operations tab shows each pod gone: the pods of web and of k3s, with no option.
    click(window, "Drain…");
    until(window, "the verdicts of the pods", |window| shows(window, "4 pods: 4 to evict, 0 to keep, 0 block"));
    click(window, "Confirm Drain");
    until(window, "the end of the drain", |window| shows(window, "4 pods: 4 gone, 0 held"));
    assert!(shows(window, "The node stays cordoned: it takes no new pod. Uncordon opens it again."));
    let cordoned = runtime.block_on(k3s.kubectl(&["get", "node", node.trim(), "-o", "jsonpath={.spec.unschedulable}"]));
    assert_eq!(cordoned.expect("the node").trim(), "true");
    stop_with_no_ended_task(window);
}
