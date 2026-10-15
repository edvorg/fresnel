//! Cordon, uncordon and drain of a Node (plan section 10): each one asks first and ends as
//! one order for the node; the drain dialog lists each pod of the node with its verdict for
//! the options of the user; the Operations tab shows each pod of a drain with its state.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::Command;
use fresnel_core::kinds::Kind;
use fresnel_core::ops::drain::{Drain, DrainOptions, DrainPod, PodState, Verdict};
use fresnel_core::ops::{JobState, Operation, Order, Outcome, StepState, Target};
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::apps::v1::DaemonSet;
use k8s_openapi::api::core::v1::Node;

use super::{and, shows_part};
use crate::data::{changed_object, scene};
use crate::fake::{self, Window, open_tab, shows, view};
use crate::kinds::scene::{live, nodes};
use crate::workloads::scene::{job, jobs, opened, runs, target};

/// The node of the scene: each pod of the scene runs on it.
fn node() -> String {
    nodes()[0].metadata.name.clone().expect("a name")
}

/// The cluster of the Nodes, with the DaemonSets that the drain dialog reads.
fn with_nodes(nodes: &[Arc<Node>]) -> fresnel_core::app::ClusterRow {
    and(live("ams-prod", nodes), &scene::<DaemonSet>())
}

fn click(window: &mut Window, label: &str) {
    window.get_by_role_and_label(Role::Button, label).scroll_to_me();
    window.run();
    window.get_by_role_and_label(Role::Button, label).click();
    window.run();
}

fn is_off(window: &Window, label: &str) -> bool {
    window.get_by_role_and_label(Role::Button, label).accesskit_node().is_disabled()
}

fn type_into(window: &mut Window, field: &str, text: &str) {
    window.get_by_role_and_label(Role::TextInput, field).click();
    window.run();
    window.key_press_modifiers(Modifiers::COMMAND, Key::A);
    window.get_by_role_and_label(Role::TextInput, field).type_text(text);
    window.run();
}

#[test]
fn cordon_and_uncordon_ask_first_and_each_one_is_off_for_a_node_in_its_state() {
    let node = node();
    let mut window = opened::<Node>(vec![with_nodes(&nodes())], &node);
    assert!(is_off(&window, "Uncordon…") && !is_off(&window, "Cordon…"), "the node takes new pods");
    click(&mut window, "Cordon…");
    assert!(shows(&window, "The scheduler puts no new pod on the node. Its pods go on to run."));
    assert!(shows(&window, "Node") && shows(&window, &node), "the target of the dialog");
    click(&mut window, "Confirm Cordon");
    let order =
        |nodes: &[Arc<Node>], operation| Order::new(target("ams-prod", nodes, &node), operation).expect("order");
    assert_eq!(runs(&window), [order(&nodes(), Operation::Cordon)]);

    let cordoned = [changed_object(&*nodes()[0], |value| value["spec"]["unschedulable"] = true.into())];
    let mut window = opened::<Node>(vec![with_nodes(&cordoned)], &node);
    assert!(is_off(&window, "Cordon…"), "a cordoned node");
    click(&mut window, "Uncordon…");
    click(&mut window, "Confirm Uncordon");
    assert_eq!(runs(&window), [order(&cordoned, Operation::Uncordon)]);
}

/// With no option, the pod of the DaemonSet and each pod with no controller that runs
/// block the drain; a pod that ended and each pod of a controller are evicted.
#[test]
fn the_drain_dialog_gives_each_pod_its_verdict_for_the_options_and_sends_them() {
    let node = node();
    let mut window = opened::<Node>(vec![with_nodes(&nodes())], &node);
    let shown = BTreeSet::from([Kind::Node, Kind::DaemonSet]);
    assert!(window.state().sent.contains(&Command::ShowKinds(shown)), "the dialog reads the DaemonSets");
    click(&mut window, "Drain…");
    assert!(shows(&window, &format!("Drain: the Node {node}")));
    assert!(shows(&window, "23 pods: 14 to evict, 0 to keep, 9 block"));
    assert!(shows(&window, "scene/agent-8djvd") && shows_part(&window, "blocks: a pod of a DaemonSet"));
    assert!(shows(&window, "scene/crash") && shows_part(&window, "blocks: a pod with no controller"));
    assert!(shows(&window, "9 pods block the drain: turn on the option that its line names."));
    assert!(is_off(&window, "Confirm Drain"));

    window.get_by_label("Ignore DaemonSets: their pods stay on the node").click();
    window.run();
    window.get_by_label("Force: evict a pod with no controller, which nothing starts again").click();
    window.run();
    assert!(shows(&window, "23 pods: 22 to evict, 1 to keep, 0 block"));
    assert!(shows(&window, "keep: a pod of a DaemonSet"));
    type_into(&mut window, "Grace period (s)", "soon");
    assert!(shows(&window, "The grace period is a count of seconds, or empty.") && is_off(&window, "Confirm Drain"));
    type_into(&mut window, "Grace period (s)", "10");
    type_into(&mut window, "Time limit (s)", "60");
    click(&mut window, "Confirm Drain");

    let options = DrainOptions {
        ignore_daemon_sets: true,
        force: true,
        grace_period: Some(10),
        timeout: Some(Duration::from_secs(60)),
        ..DrainOptions::default()
    };
    let order = Order::new(target("ams-prod", &nodes(), &node), Operation::Drain(options)).expect("an order");
    assert_eq!(runs(&window), [order]);
    let note = format!(
        "Started: Drain --ignore-daemonsets --force --grace-period=10 --timeout=60s of Node {node} in ams-prod. \
         The Operations tab shows each pod."
    );
    assert!(shows(&window, &note));
}

/// A drain that runs and one that the user cancelled, on the Operations tab.
fn drain_job(number: u64, state: JobState) -> Arc<fresnel_core::app::JobView> {
    let node: Target = target("ams-prod", &nodes(), &node());
    let pod = |name: &str, verdict, state| DrainPod { key: ObjectKey::new("shop", name), verdict, state };
    let budget = "The disruption budget guarded needs 1 healthy pods and has 1 currently";
    let pods = [
        pod("agent-x1", Verdict::Keep("a pod of a DaemonSet"), PodState::Kept),
        pod("web-1", Verdict::Evict(None), PodState::Gone),
        pod(
            "guarded-1",
            Verdict::Evict(None),
            PodState::Blocked { budget: Some("guarded".to_owned()), reason: budget.to_owned(), attempts: 3 },
        ),
        pod("lone", Verdict::Evict(Some("it has no controller: nothing starts it again")), PodState::Terminating),
    ];
    let steps = [("cordon the node", StepState::Done("the node takes no new pod".to_owned()))];
    let options = DrainOptions { ignore_daemon_sets: true, force: true, ..DrainOptions::default() };
    let mut job = job(number, node, Operation::Drain(options), state, &steps);
    Arc::make_mut(&mut job).report.outcome = Outcome::Drain(Drain { cordoned: true, written: None, pods: pods.into() });
    job
}

#[test]
fn the_operations_tab_shows_each_pod_of_a_drain_and_a_budget_by_its_name_and_a_cancel_keeps_the_cordon() {
    let mut core = view(vec![with_nodes(&nodes())]);
    core.jobs = jobs(vec![drain_job(1, JobState::Pending)], 0);
    let mut window = fake::window(core);
    open_tab(&mut window, "Operations");
    assert!(shows(&window, "Job 1: Drain --ignore-daemonsets --force"));
    assert!(shows(&window, "4 pods: 1 gone, 1 held"));
    let held = "held by the PodDisruptionBudget guarded (3×): The disruption budget guarded needs 1 healthy pods";
    assert!(shows_part(&window, held), "the budget by its name");
    for text in ["shop/guarded-1", "kept on the node", "gone", "terminating", "keep: a pod of a DaemonSet"] {
        assert!(shows(&window, text), "{text}");
    }
    assert!(!shows_part(&window, "stays cordoned"), "the drain runs");
    window.get_by_label("Cancel job 1").click();
    window.run();
    assert_eq!(window.state().sent.last(), Some(&Command::Cancel(1)));

    let mut core = view(vec![with_nodes(&nodes())]);
    core.jobs = jobs(vec![drain_job(1, JobState::Cancelled)], 0);
    let mut window = fake::window(core);
    open_tab(&mut window, "Operations");
    let text = "Cancelled: no new eviction goes out. The node stays cordoned; Uncordon opens it again.";
    assert!(shows(&window, text));
    assert!(window.query_by_label("Cancel job 1").is_none());
}
