//! The tab that creates or applies a text of documents (plan section 12.9): its key opens it,
//! its dialog lists each document as cluster, namespace, kind and name, one command sends
//! the documents in turn, the tab shows the state of each job, and a conflict of an apply
//! asks before a force. A read-only cluster keeps it off.

use std::sync::Arc;

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::CoreError;
use fresnel_core::app::{Command, JobView, JobsView};
use fresnel_core::kinds::Kind;
use fresnel_core::net::ErrorClass;
use fresnel_core::ops::{JobState, Operation, Order, Report};

use super::checked;
use crate::fake::{Window, shows};
use crate::settings::with_config;

/// A ConfigMap and a Deployment with no namespace, and a Secret of an other namespace.
pub(crate) const TEXT: &str = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: one\ndata:\n  mode: fast\n---\n\
                               apiVersion: v1\nkind: Secret\nmetadata:\n  name: two\n  namespace: other\n---\n\
                               apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: three\n";

/// The window with a tab of a create from its key, with `TEXT` and the target namespace `shop`.
pub(crate) fn tab(window: &mut Window) {
    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::I);
    window.run();
    assert!(shows(window, "Create or apply YAML") && shows(window, "YAML"), "the tab of the key");
    window.event(Event::Paste(TEXT.to_owned()));
    window.run();
    let field = window.get_by_label("Target namespace of the YAML");
    field.click();
    window.run();
    window.key_press_modifiers(Modifiers::COMMAND, Key::A);
    window.key_press(Key::Backspace);
    window.get_by_label("Target namespace of the YAML").type_text("shop");
    window.run();
    checked(window, "No error");
}

/// The orders of the last `Command::RunInTurn` of the views.
pub(crate) fn in_turn(window: &Window) -> Vec<Order> {
    let mut sent = window.state().sent.iter().rev();
    let orders = sent.find_map(|command| match command {
        Command::RunInTurn(orders) => Some(orders.clone()),
        _ => None,
    });
    orders.expect("a run in turn")
}

/// Gives the fake core these jobs, from the number 1 on.
pub(crate) fn jobs(window: &mut Window, jobs: Vec<(Order, JobState)>) {
    let jobs = jobs.into_iter().enumerate().map(|(index, (order, state))| {
        Arc::new(JobView { number: index as u64 + 1, order, report: Report { state, ..Report::default() } })
    });
    window.state_mut().view.jobs = Arc::new(JobsView { jobs: jobs.collect(), dropped: 0 });
    window.run();
}

#[test]
fn a_create_lists_each_document_with_its_target_and_sends_them_in_turn() {
    let mut window = super::panel();
    tab(&mut window);
    let top = |window: &Window| window.get_by_label("Operations").rect().top();
    let before = top(&window);
    window.get_by_label("Create the documents").click();
    window.run();
    assert!(shows(&window, "Create 2 objects in ams-prod?"));
    for text in ["ConfigMap shop/one", "Deployment shop/three"] {
        assert!(shows(&window, text), "{text}");
    }
    let other = "Secret two: Its namespace other is not the target namespace shop. Choose other to send it.";
    assert!(shows(&window, other));
    window.get_by_label("Create 2 objects").click();
    window.run();
    let orders = in_turn(&window);
    let targets: Vec<(&str, Kind, &str, &str)> = orders
        .iter()
        .map(|order| {
            let target = &order.target;
            (target.cluster.as_str(), target.kind, &*target.key.namespace, &*target.key.name)
        })
        .collect();
    assert_eq!(
        targets,
        [("ams-prod", Kind::ConfigMap, "shop", "one"), ("ams-prod", Kind::Deployment, "shop", "three")]
    );
    assert!(orders.iter().all(|order| matches!(order.operation, Operation::Create(_))));
    assert!(window.get_by_label("Create the documents").accesskit_node().is_disabled(), "the run waits for its jobs");

    jobs(&mut window, vec![(orders[0].clone(), JobState::Done), (orders[1].clone(), JobState::Pending)]);
    assert!(shows(&window, "Create: 1 of 2 jobs ended. The Operations tab has their steps."));
    assert!(shows(&window, "done") && shows(&window, "running"));
    for _ in 0..10 {
        window.run();
    }
    assert_eq!(top(&window), before, "the documents of the run keep the height of the dock");
}

#[test]
fn an_apply_with_a_conflict_names_its_fields_and_asks_before_a_force() {
    let mut window = super::panel();
    tab(&mut window);
    window.get_by_label("Apply the documents").click();
    window.run();
    window.get_by_label("Apply 2 objects").click();
    window.run();
    let orders = in_turn(&window);
    assert!(orders.iter().all(|order| matches!(order.operation, Operation::Apply { force: false, .. })));
    let field = r#".data.mode (conflict with "kubectl-create" using v1)"#;
    let conflict = CoreError::new(ErrorClass::Conflict, "Apply failed with 1 conflict (HTTP 409)")
        .with_causes(vec![("FieldManagerConflict".to_owned(), field.to_owned())]);
    jobs(&mut window, vec![(orders[0].clone(), JobState::Failed(conflict)), (orders[1].clone(), JobState::Done)]);
    assert!(shows(&window, "failed: Apply failed with 1 conflict (HTTP 409)"));
    window.get_by_label("Apply with force").click();
    window.run();
    assert!(shows(&window, "Apply 1 object with force to ams-prod?"));
    assert!(shows(&window, &format!("   {field}")), "the field and its manager");
    window.get_by_label("Apply with force 1 object").click();
    window.run();
    let force = in_turn(&window);
    assert_eq!(force.len(), 1);
    assert_eq!((force[0].target.kind, &*force[0].target.key.name), (Kind::ConfigMap, "one"));
    assert!(matches!(force[0].operation, Operation::Apply { force: true, .. }));
}

#[test]
fn a_read_only_cluster_keeps_the_create_and_the_apply_off() {
    let mut window = super::panel();
    let settings = "version = 1\n[clusters.ams-prod]\nread_only = true\n";
    window.state_mut().view = with_config(window.state().view.clone(), settings);
    tab(&mut window);
    for button in ["Create the documents", "Apply the documents"] {
        assert!(window.get_by_label(button).accesskit_node().is_disabled(), "{button}");
    }
    assert!(shows(&window, "ams-prod is read-only in the settings: the app makes no change in it."));
}
