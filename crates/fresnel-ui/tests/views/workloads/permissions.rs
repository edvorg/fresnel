//! The permissions of the user (plan section 10): an action that the rules forbid is off
//! with its reason in the header, in the menu, for a key and in the dialog. The view asks
//! the core for the rules of a namespace that it does not know, and for an access review of
//! an action on the cluster.

use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::{AccessView, Command};
use fresnel_core::cluster::ClusterId;
use fresnel_core::ops::access::Attributes;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Node;

use super::scene::{access, cluster, hover, opened, rules_without, runs};
use crate::data::scene;
use crate::fake::{Window, shows};
use crate::kinds::scene::nodes;

fn is_off(window: &Window, button: &str) -> bool {
    window.get_by_role_and_label(Role::Button, button).accesskit_node().is_disabled()
}

#[test]
fn a_forbidden_action_is_off_with_its_reason_and_the_others_stay_on() {
    let deployments = scene::<Deployment>();
    let rules = access(rules_without(&[("patch", "deployments/scale"), ("delete", "deployments")]));
    let mut window = opened::<Deployment>(vec![cluster("ams-prod", &deployments, rules)], "web");
    assert!(is_off(&window, "Scale…") && is_off(&window, "Delete…"));
    assert!(!is_off(&window, "Restart…") && !is_off(&window, "Rollout status"), "the rules permit them");

    // The reason shows at the pointer.
    hover(&mut window, Role::Button, "Scale…");
    let reason = "Forbidden: no rule lets you patch deployments/scale (apps) web in scene.";
    assert!(shows(&window, reason), "the hover text of the button");

    // The menu has the same gate, and so has the menu of a row.
    window.get_by_role_and_label(Role::Button, "More").click();
    window.run();
    assert!(window.get_by_label("Force delete…").accesskit_node().is_disabled(), "a force delete is a delete");
    assert!(!window.get_by_label_contains("Copy name").accesskit_node().is_disabled());
    assert_eq!(runs(&window), []);
}

/// A key of an action that the rules forbid runs nothing and says why.
#[test]
fn a_key_of_a_forbidden_action_gives_a_note_with_its_reason() {
    use fresnel_core::commands::{CommandId, Keys, Shortcut};
    use fresnel_core::config::Config;
    use fresnel_core::kinds::Action;

    let deployments = scene::<Deployment>();
    let rules = access(rules_without(&[("patch", "deployments/scale")]));
    let shortcut = Shortcut::parse("Alt+S").expect("a shortcut");
    let (keys, _) = Keys::with_choices(&[(CommandId::Object(Action::Scale), Some(shortcut), ())]);
    let mut session = crate::fake::view(vec![cluster("ams-prod", &deployments, rules)]);
    session.config = Arc::new(Config { keys, ..Config::default() });
    let mut window = crate::fake::window(session);
    super::scene::open(&mut window, "Deployments", "web");
    window.key_press_modifiers(egui::Modifiers::ALT, egui::Key::S);
    window.run();
    let note = "Scale did not run: Forbidden: no rule lets you patch deployments/scale (apps) web in scene.";
    assert!(shows(&window, note));
    assert!(window.query_by_label_contains("The action acts on each").is_none(), "no dialog");
}

/// With no rules of the namespace, the panel asks the core for them one time, and the
/// actions stay on: the server decides, and the core handles its HTTP 403.
#[test]
fn the_panel_asks_for_the_rules_of_its_namespace_one_time_and_keeps_the_actions_on() {
    let deployments = scene::<Deployment>();
    let mut window = opened::<Deployment>(vec![cluster("ams-prod", &deployments, Arc::default())], "web");
    let asked = |window: &Window| {
        let read = |command: &&Command| matches!(command, Command::ReadRules { cluster, namespace } if cluster.as_str() == "ams-prod" && &**namespace == "scene");
        window.state().sent.iter().filter(read).count()
    };
    assert_eq!(asked(&window), 1, "one question for each namespace");
    window.run();
    assert_eq!(asked(&window), 1, "the next frames do not ask again");
    assert!(!is_off(&window, "Scale…"));
    hover(&mut window, Role::Button, "Scale…");
    assert!(window.query_by_label_contains("The server decides").is_some());
}

/// A Node has no namespace: the rules of a namespace cannot tell, so the panel asks the
/// server with an access review for each request of an action.
#[test]
fn an_action_on_an_object_of_the_cluster_asks_for_an_access_review() {
    let nodes = nodes();
    let name = nodes.first().and_then(|node| node.metadata.name.clone()).expect("a name");
    let window = opened::<Node>(vec![cluster("ams-prod", &nodes, Arc::new(AccessView::default()))], &name);
    let wanted = Attributes {
        verb: "delete",
        group: "",
        resource: "nodes",
        subresource: "",
        namespace: "".into(),
        name: name.as_str().into(),
    };
    let check = Command::CheckAccess { cluster: ClusterId::new("ams-prod"), attributes: wanted };
    assert!(window.state().sent.contains(&check), "{:?}", window.state().sent);
}

/// The dialog decides each target in each frame: a target whose permission goes away
/// while the dialog is open shows its reason, and the button is off.
#[test]
fn a_target_that_becomes_forbidden_in_the_dialog_turns_its_button_off() {
    let deployments = scene::<Deployment>();
    let rules = access(rules_without(&[]));
    let mut window = opened::<Deployment>(vec![cluster("ams-prod", &deployments, rules)], "web");
    window.get_by_role_and_label(Role::Button, "Restart…").click();
    window.run();
    assert!(!window.get_by_label("Confirm Restart").accesskit_node().is_disabled());
    let denied = access(rules_without(&[("patch", "deployments")]));
    window.state_mut().view = crate::fake::view(vec![cluster("ams-prod", &deployments, denied)]);
    window.run();
    assert!(window.get_by_label("Confirm Restart").accesskit_node().is_disabled());
    let reason = "Deployment scene/web in ams-prod: Forbidden: no rule lets you patch deployments (apps) web in scene.";
    assert!(shows(&window, reason));
}
