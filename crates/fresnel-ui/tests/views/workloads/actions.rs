//! Each action from the one command table (plan section 11.5): the header of the panel,
//! the menu of a row, a key and the quick search. An action that changes a cluster asks
//! first in a dialog that lists each target as cluster, namespace, kind and name, and its
//! end is one order for its object (plan section 11.2).

use std::sync::Arc;

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::commands::{CommandId, Keys, Shortcut};
use fresnel_core::config::Config;
use fresnel_core::kinds::Action;
use fresnel_core::ops::{JobState, Operation, Order, Outcome, Propagation, Revision};
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Pod, ReplicationController};
use serde_json::Value;

use super::scene::{allowed, job, jobs, open, opened, runs, target};
use crate::data::{of_kinds, pods, scene};
use crate::fake::{Window, shows, until, view, window};
use crate::kinds::in_panel;

/// Clicks a button of the window and draws the frames after it. The panel scrolls to a
/// button below its bottom first.
fn click(window: &mut Window, label: &str) {
    window.get_by_role_and_label(Role::Button, label).scroll_to_me();
    window.run();
    window.get_by_role_and_label(Role::Button, label).click();
    window.run();
}

/// True while the confirm dialog shows these texts of its target: cluster, namespace,
/// kind and name.
fn lists(window: &Window, texts: &[&str]) -> bool {
    window.query_by_label_contains("The action acts on each of these objects").is_some()
        && texts.iter().all(|text| window.query_all_by_label(text).next().is_some())
}

/// Clicks an action of the header, sees its target in the dialog and confirms it.
fn confirm(window: &mut Window, button: &str, target: &[&str]) {
    click(window, button);
    assert!(lists(window, target), "the dialog of {button} lists {target:?}");
    let label = button.trim_end_matches('…');
    click(window, &format!("Confirm {label}"));
    assert!(window.query_by_label_contains("The action acts on each").is_none(), "the dialog closed");
}

/// Replaces the text of a field.
fn type_into(window: &mut Window, field: &str, text: &str) {
    window.get_by_role_and_label(Role::TextInput, field).click();
    window.run();
    window.key_press_modifiers(Modifiers::COMMAND, Key::A);
    window.get_by_role_and_label(Role::TextInput, field).type_text(text);
    window.run();
}

/// The text of a field.
fn text_of(window: &Window, field: &str) -> String {
    window.get_by_role_and_label(Role::TextInput, field).value().unwrap_or_default()
}

#[test]
fn each_operation_of_a_deployment_asks_first_and_sends_one_order_for_its_object() {
    let deployments = scene::<Deployment>();
    let mut window = opened::<Deployment>(vec![allowed("ams-prod", &deployments)], "web");
    let web = target("ams-prod", &deployments, "web");
    let order = |operation| Order::new(web.clone(), operation).expect("an order");
    let what = ["ams-prod", "scene", "Deployment", "web"];

    click(&mut window, "Scale…");
    assert!(lists(&window, &what));
    assert_eq!(text_of(&window, "Count of pods"), "2", "the count of now");
    type_into(&mut window, "Count of pods", "five");
    assert!(window.get_by_label("Confirm Scale").accesskit_node().is_disabled(), "not a number");
    assert!(shows(&window, "The count of pods is a number of 0 or more."));
    type_into(&mut window, "Count of pods", "5");
    click(&mut window, "Confirm Scale");
    assert_eq!(runs(&window), [order(Operation::Scale(5))]);
    assert!(shows(
        &window,
        "Started: Scale to 5 of Deployment scene/web in ams-prod. The Operations tab shows the job."
    ));

    confirm(&mut window, "Restart…", &what);
    confirm(&mut window, "Roll back…", &what);
    click(&mut window, "Delete…");
    window.get_by_label("Keep the dependents, with no owner").click();
    window.run();
    assert!(shows(&window, "The dependents stay and run on, with no owner."));
    click(&mut window, "Confirm Delete");
    // The read actions need no dialog.
    click(&mut window, "Rollout status");
    click(&mut window, "Revision history");
    let expected = [
        Operation::Scale(5),
        Operation::Restart,
        Operation::Rollback(0),
        Operation::Delete(Propagation::Orphan),
        Operation::RolloutStatus,
        Operation::History,
    ];
    assert_eq!(runs(&window), expected.map(order));
    assert!(window.query_by_label("Count of pods").is_none());
}

#[test]
fn cancel_and_escape_close_a_dialog_and_send_nothing() {
    let deployments = scene::<Deployment>();
    let mut window = opened::<Deployment>(vec![allowed("ams-prod", &deployments)], "web");
    click(&mut window, "Restart…");
    click(&mut window, "Cancel");
    click(&mut window, "Delete…");
    window.key_press(Key::Escape);
    window.run();
    assert!(window.query_by_label_contains("The action acts on each").is_none());
    assert_eq!(runs(&window), []);
}

#[test]
fn a_pod_evicts_deletes_and_force_deletes_with_the_warning_of_a_stateful_set() {
    let pods = pods();
    let mut window = opened::<Pod>(vec![allowed("ams-prod", &pods)], "db-0");
    let what = ["ams-prod", "scene", "Pod", "db-0"];
    click(&mut window, "Evict…");
    assert!(shows(&window, "The eviction obeys each PodDisruptionBudget: a budget can refuse it."));
    click(&mut window, "Confirm Evict");
    confirm(&mut window, "Delete…", &what);
    click(&mut window, "More");
    window.get_by_label("Force delete…").click();
    window.run();
    assert!(lists(&window, &what));
    let warning = "A pod of a StatefulSet: a new pod with the same identity can start while the old one runs.";
    assert!(shows(&window, warning));
    click(&mut window, "Confirm Force delete");
    let db = target("ams-prod", &pods, "db-0");
    let expected = [Operation::Evict, Operation::Delete(Propagation::Background), Operation::ForceDelete];
    assert_eq!(runs(&window), expected.map(|operation| Order::new(db.clone(), operation).expect("an order")));
}

#[test]
fn a_job_runs_again_and_a_suspended_job_resumes_but_does_not_suspend_again() {
    let jobs = scene::<Job>();
    let mut window = opened::<Job>(vec![allowed("ams-prod", &jobs)], "loop-b");
    let suspend = window.get_by_role_and_label(Role::Button, "Suspend…");
    assert!(suspend.accesskit_node().is_disabled(), "it is suspended already");
    confirm(&mut window, "Resume…", &["Job", "loop-b"]);
    click(&mut window, "Run again…");
    assert!(shows(&window, "The Job did not end: its pods stop now."), "loop-b did not end");
    click(&mut window, "Confirm Run again");
    let loop_b = target("ams-prod", &jobs, "loop-b");
    let expected = [Operation::Resume, Operation::RunAgain];
    assert_eq!(runs(&window), expected.map(|operation| Order::new(loop_b.clone(), operation).expect("an order")));
}

#[test]
fn a_cron_job_triggers_a_job_with_a_name_that_the_server_accepts() {
    let cron_jobs = scene::<CronJob>();
    let mut window = opened::<CronJob>(vec![allowed("ams-prod", &cron_jobs)], "tick");
    click(&mut window, "Trigger now…");
    let name = text_of(&window, "Name of the Job");
    assert!(name.starts_with("tick-manual-") && name.len() == "tick-manual-".len() + 5, "{name}");
    type_into(&mut window, "Name of the Job", "Tick");
    assert!(window.get_by_label("Confirm Trigger now").accesskit_node().is_disabled());
    type_into(&mut window, "Name of the Job", "tick-by-hand");
    click(&mut window, "Confirm Trigger now");
    confirm(&mut window, "Suspend…", &["CronJob", "tick"]);
    let tick = target("ams-prod", &cron_jobs, "tick");
    let expected = [Operation::Trigger("tick-by-hand".to_owned()), Operation::Suspend];
    assert_eq!(runs(&window), expected.map(|operation| Order::new(tick.clone(), operation).expect("an order")));
}

/// The other workloads: a scale of a StatefulSet, a ReplicaSet and a ReplicationController,
/// and a restart and a rollback of a DaemonSet, which has no scale.
#[test]
fn the_other_workloads_scale_restart_and_roll_back_by_their_kind() {
    let sets = scene::<StatefulSet>();
    let mut window = opened::<StatefulSet>(vec![allowed("ams-prod", &sets)], "db");
    confirm(&mut window, "Scale…", &["StatefulSet", "db"]);
    assert_eq!(runs(&window), [Order::new(target("ams-prod", &sets, "db"), Operation::Scale(2)).expect("an order")]);

    let daemons = scene::<DaemonSet>();
    let mut window = opened::<DaemonSet>(vec![allowed("ams-prod", &daemons)], "agent");
    assert!(window.query_by_label("Scale…").is_none(), "a DaemonSet has no scale");
    confirm(&mut window, "Restart…", &["DaemonSet", "agent"]);
    confirm(&mut window, "Roll back…", &["DaemonSet", "agent"]);
    assert_eq!(runs(&window).len(), 2);

    let replica_sets = scene::<ReplicaSet>();
    let mut window = opened::<ReplicaSet>(vec![allowed("ams-prod", &replica_sets)], "web-6888b778b6");
    confirm(&mut window, "Scale…", &["ReplicaSet", "web-6888b778b6"]);
    let controllers = of_kinds::<ReplicationController>();
    let mut window = opened::<ReplicationController>(vec![allowed("ams-prod", &controllers)], "legacy");
    confirm(&mut window, "Scale…", &["ReplicationController", "legacy"]);
    assert!(matches!(runs(&window).as_slice(), [order] if matches!(order.operation, Operation::Scale(_))));
}

/// A rollback to one revision of the list of the revisions, after a read of the revisions.
#[test]
fn a_revision_of_the_list_rolls_back_to_that_revision() {
    let deployments = scene::<Deployment>();
    let web = target("ams-prod", &deployments, "web");
    let revision = |number, current| Revision {
        number,
        name: format!("web-{number}"),
        created: None,
        change_cause: Some(format!("round {number}")),
        images: vec!["rancher/mirrored-pause:3.6".to_owned()],
        current,
        template: Arc::new(Value::Null),
    };
    let mut read = job(4, web.clone(), Operation::History, JobState::Done, &[]);
    Arc::make_mut(&mut read).report.outcome = Outcome::Revisions(vec![revision(1, false), revision(2, true)].into());
    let mut session = view(vec![allowed("ams-prod", &deployments)]);
    session.jobs = jobs(vec![read], 0);
    let mut window = window(session);
    open(&mut window, "Deployments", "web");
    assert!(in_panel(&window, "round 1") && in_panel(&window, "current"));
    click(&mut window, "Roll back to 1…");
    assert!(shows(&window, "Roll back to revision 1"), "the revision of the click is a choice");
    click(&mut window, "Confirm Roll back");
    assert_eq!(runs(&window), [Order::new(web, Operation::Rollback(1)).expect("an order")]);
}

/// A fast double click on "Read again" gives a second read that the core does not send while
/// the first one runs. That second job is a note: the panel keeps the read that runs, and
/// then its revisions.
#[test]
fn a_second_read_of_the_revisions_that_the_core_did_not_send_hides_no_read() {
    let deployments = scene::<Deployment>();
    let web = target("ams-prod", &deployments, "web");
    let not_sent = || {
        let error = crate::fake::error("job 4 runs the same operation on the object");
        job(5, web.clone(), Operation::History, JobState::NotSent(error), &[])
    };
    let mut session = view(vec![allowed("ams-prod", &deployments)]);
    session.jobs = jobs(vec![job(4, web.clone(), Operation::History, JobState::Pending, &[]), not_sent()], 0);
    let mut window = window(session);
    open(&mut window, "Deployments", "web");
    assert!(in_panel(&window, "Job 4 reads the revisions…"));
    let note = "Job 5 did not read the revisions: job 4 runs the same operation on the object";
    assert!(in_panel(&window, note));

    let mut read = job(4, web.clone(), Operation::History, JobState::Done, &[]);
    let revision = Revision {
        number: 1,
        name: "web-1".to_owned(),
        created: None,
        change_cause: Some("x".repeat(100_000)),
        images: vec!["rancher/mirrored-pause:3.6".to_owned()],
        current: true,
        template: Arc::new(Value::Null),
    };
    Arc::make_mut(&mut read).report.outcome = Outcome::Revisions(vec![revision].into());
    window.state_mut().view.jobs = jobs(vec![read, not_sent()], 0);
    window.run();
    assert!(in_panel(&window, "current") && in_panel(&window, note));
    // The change cause is an annotation of any length: the panel shows its start.
    let cause = window.query_all_by_label_contains("xxxx").next().and_then(|node| node.value()).expect("the cause");
    assert!(cause.chars().count() <= 241, "{} characters", cause.chars().count());
}

/// The same action from a key of the settings, from the menu of a row, and from the quick
/// search: each one opens the same dialog for the object of the panel.
#[test]
fn a_key_a_menu_and_the_quick_search_open_the_same_dialog() {
    let deployments = scene::<Deployment>();
    let shortcut = Shortcut::parse("Alt+R").expect("a shortcut");
    let (keys, _) = Keys::with_choices(&[(CommandId::Object(Action::Restart), Some(shortcut), ())]);
    let mut session = view(vec![allowed("ams-prod", &deployments)]);
    session.config = Arc::new(Config { keys, ..Config::default() });
    let mut window = window(session);
    open(&mut window, "Deployments", "web");
    window.key_press_modifiers(Modifiers::ALT, Key::R);
    window.run();
    assert!(lists(&window, &["Deployment", "web"]), "the key");
    click(&mut window, "Cancel");

    // With no panel, the menu is the one place with the action.
    click(&mut window, "Close the details");
    window.get_by_role_and_label(Role::Label, "web").click_secondary();
    window.run();
    window.get_by_label_contains("Restart…").click();
    window.run();
    assert!(lists(&window, &["Deployment", "web"]), "the menu acts on its row");
    click(&mut window, "Cancel");

    window.key_press_modifiers(Modifiers::COMMAND, Key::K);
    window.run();
    window.get_by_label("Search text").type_text("Restart");
    until(&mut window, "the command", |window| window.query_by_label("Command Restart").is_some());
    window.key_press(Key::Enter);
    window.run();
    assert!(lists(&window, &["Deployment", "web"]), "the quick search acts on the object of the panel");
}

/// "Copy kubectl command" lists the commands of the object; each one names its context. The
/// logs have their own view in the app (M9).
#[test]
fn the_kubectl_commands_of_a_deployment_have_its_port_forward_and_no_logs() {
    let deployments = scene::<Deployment>();
    let mut window = opened::<Deployment>(vec![allowed("ams-prod", &deployments)], "web");
    click(&mut window, "More");
    window.get_by_label_contains("Copy kubectl command").click();
    window.run();
    let lines = [
        "kubectl --context ams-prod edit deployments.apps --namespace scene web",
        "kubectl --context ams-prod port-forward --namespace scene deployments.apps/web 8080:80",
    ];
    for line in lines {
        assert!(shows(&window, line), "{line}");
    }
    assert!(window.query_by_label_contains(" logs ").is_none(), "no kubectl logs");
    click(&mut window, "Copy port-forward 80");
    assert!(shows(&window, &format!("Copied: {}", lines[1])));
}
