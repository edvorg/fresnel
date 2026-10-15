//! A read-only cluster has each action that changes it off, with the reason, and the
//! confirm rule `type-name` keeps the confirm button off until the user typed the name of
//! the cluster: also of a key save and of a new Secret.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::Command;
use fresnel_core::keys::{KeyKind, Place};
use fresnel_core::kinds::Action;
use fresnel_core::ops::Operation;
use k8s_openapi::api::apps::v1::Deployment;

use super::with_config;
use crate::data::scene;
use crate::fake::{Window, shows, view, window};
use crate::keys::scene::{give, loaded, read, secret, target, with_secrets};
use crate::keys::{click, save_command, typed};
use crate::workloads::scene::{allowed, hover, open, runs};

fn is_off(window: &Window, button: &str) -> bool {
    window.get_by_role_and_label(Role::Button, button).accesskit_node().is_disabled()
}

const READ_ONLY: &str = "ams-prod is read-only in the settings: the app makes no change in it.";

#[test]
fn a_read_only_cluster_has_each_change_off_with_its_reason_and_its_reads_on() {
    let deployments = scene::<Deployment>();
    let clusters = vec![allowed("ams-prod", &deployments), allowed("fra-prod", &deployments)];
    let text = "version = 1\n[clusters.ams-prod]\nread_only = true\n";
    let mut window = window(with_config(view(clusters), text));
    open(&mut window, "Deployments", "web");
    // The filter shows web of both clusters; the panel of ams-prod opened first.
    assert!(shows(&window, "ams-prod"));
    assert!(is_off(&window, "Scale…") && is_off(&window, "Restart…") && is_off(&window, "Delete…"));
    assert!(!is_off(&window, "Rollout status") && !is_off(&window, "Revision history"), "a read stays on");
    hover(&mut window, Role::Button, "Scale…");
    assert!(shows(&window, READ_ONLY), "the reason at the pointer");

    window.get_by_role_and_label(Role::Button, "More").click();
    window.run();
    assert!(window.get_by_label("Force delete…").accesskit_node().is_disabled());
    assert!(!window.get_by_label_contains("Copy name").accesskit_node().is_disabled());
    window.key_press(egui::Key::Escape);
    window.run();
    // The key of a change gives the reason as a note, and no dialog.
    window.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::R);
    window.run();
    assert!(shows(&window, &format!("Restart did not run: {READ_ONLY}")), "the note of the key");
    assert_eq!(runs(&window), []);
}

#[test]
fn the_rule_type_name_keeps_the_confirm_off_until_the_name_of_the_cluster_is_typed() {
    let deployments = scene::<Deployment>();
    let text = "version = 1\n[clusters.ams-prod]\nconfirm = \"type-name\"\n";
    let mut window = window(with_config(view(vec![allowed("ams-prod", &deployments)]), text));
    open(&mut window, "Deployments", "web");
    window.get_by_role_and_label(Role::Button, "Restart…").click();
    window.run();
    assert!(shows(&window, "Type ams-prod to confirm a change in it:"));
    assert!(window.get_by_label("Confirm Restart").accesskit_node().is_disabled());
    assert!(shows(&window, "The settings ask for the name of ams-prod before a change."));
    window.get_by_label("Name of the cluster").click();
    window.run();
    window.get_by_label("Name of the cluster").type_text("ams-pro");
    window.run();
    assert!(window.get_by_label("Confirm Restart").accesskit_node().is_disabled(), "a part of the name");
    window.get_by_label("Name of the cluster").type_text("d");
    window.run();
    window.get_by_label("Confirm Restart").click();
    window.run();
    let operations: Vec<Operation> = runs(&window).into_iter().map(|order| order.operation).collect();
    assert_eq!(operations, [Operation::Restart]);
    assert_eq!(Action::Restart.label(), "Restart");
}

/// The rule asks before each change of its cluster: a save of the keys of a Secret and a
/// new Secret too, which have dialogs of their own.
#[test]
fn the_rule_type_name_asks_before_a_key_save_and_a_new_secret_too() {
    let secrets = [secret("db", "Opaque", &["password"])];
    let text = "version = 1\n[clusters.ams-prod]\nconfirm = \"type-name\"\n";
    let mut window = window(with_config(view(vec![with_secrets("ams-prod", &secrets)]), text));
    crate::keys::scene::open(&mut window, "Secrets", "db");
    let db = target("ams-prod", KeyKind::Secret, "db");
    give(&mut window, "ams-prod", Some(loaded(read(&db, "4711", &[("password", Place::Data, b"made-up")]))));
    click(&mut window, "Show the value of password");
    typed(&mut window, "Value of password", "made-up-2");
    click(&mut window, "Save 1 key…");
    assert!(shows(&window, "Type ams-prod to confirm a change in it:"));
    assert!(is_off(&window, "Save 1 key") && save_command(&window).is_none(), "the save waits for the name");
    typed(&mut window, "Name of the cluster", "ams-prod");
    click(&mut window, "Save 1 key");
    assert!(save_command(&window).is_some_and(|save| save.target == db), "the save after the name");

    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
    window.run();
    typed(&mut window, "Name of the new Secret", "api");
    typed(&mut window, "Key 1", "token");
    typed(&mut window, "Value 1", "made-up");
    assert!(shows(&window, "The settings ask for the name of ams-prod before a change."));
    assert!(window.get_by_label("Create the Secret").accesskit_node().is_disabled());
    typed(&mut window, "Name of the cluster", "ams-prod");
    click(&mut window, "Create the Secret");
    let created = window.state().sent.iter().any(|command| matches!(command, Command::CreateSecret(_)));
    assert!(created, "the create after the name");
}
