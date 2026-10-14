//! Commands, keys and navigation (plan section 11.5): Back and Forward through the visited
//! objects, an object that a filter hides, the keys of the settings, the menu of a row and
//! the namespace filter of each view.

use std::sync::Arc;

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::commands::{CommandId, Keys};
use fresnel_core::config::Config;
use fresnel_core::store::StoreState;

use crate::fake::{Window, open_page, shows, until, view, window};
use crate::kinds::in_panel;
use crate::kinds::scene::{config_maps, live as live_kind, nodes};
use crate::pods::column;
use crate::search::{search, searchable, until_hit};

fn back(window: &mut Window) {
    window.key_press_modifiers(Modifiers::COMMAND, Key::OpenBracket);
    window.run();
}

fn forward(window: &mut Window) {
    window.key_press_modifiers(Modifiers::COMMAND, Key::CloseBracket);
    window.run();
}

fn click_row(window: &mut Window, name: &str) {
    window.get_by_label(name).click();
    window.run();
}

#[test]
fn back_and_forward_move_through_the_visited_objects_of_each_kind() {
    let mut window = window(view(vec![searchable("ams-prod", StoreState::Live)]));
    click_row(&mut window, "crash");
    click_row(&mut window, "failed");
    search(&mut window, "web-secret");
    until_hit(&mut window, "Secret kinds/web-secret of ams-prod");
    window.key_press(Key::Enter);
    until(&mut window, "the panel of the Secret", |window| in_panel(window, "web-secret"));

    back(&mut window);
    until(&mut window, "the pod before the Secret", |window| in_panel(window, "failed"));
    assert!(shows(&window, "25 pods"), "the page of the Pods");
    back(&mut window);
    until(&mut window, "the first pod", |window| in_panel(window, "crash"));
    back(&mut window);
    assert!(in_panel(&window, "crash"), "no object before the first one");

    forward(&mut window);
    until(&mut window, "the second pod", |window| in_panel(window, "failed"));
    forward(&mut window);
    until(&mut window, "the Secret again", |window| in_panel(window, "web-secret"));

    // A visit after a step back drops the objects after it.
    back(&mut window);
    until(&mut window, "the second pod", |window| in_panel(window, "failed"));
    click_row(&mut window, "done");
    forward(&mut window);
    assert!(in_panel(&window, "done"), "no object after the new visit");
}

/// Plan section 11.5: a filter that hides the row of an object that a link opens gives a
/// note with an action that shows the row.
#[test]
fn an_object_that_a_filter_hides_gets_a_note_and_show_it_clears_the_filter() {
    let mut window = window(view(vec![searchable("ams-prod", StoreState::Live)]));
    window.get_by_label("Filter pods").click();
    window.run();
    window.get_by_label("Filter pods").type_text("crash");
    until(&mut window, "the rows of the filter", |window| shows(window, "2 of 25 pods"));

    search(&mut window, "half-done");
    until_hit(&mut window, "Pod scene/half-done of ams-prod");
    window.key_press(Key::Enter);
    until(&mut window, "the note of the filter", |window| shows(window, "The filters hide the row of this pod."));
    assert!(in_panel(&window, "half-done"), "the panel shows the object");

    window.get_by_label("Show it").click();
    until(&mut window, "each row", |window| shows(window, "25 pods"));
    assert!(column(&window, "Name").contains(&"half-done".to_owned()));
    assert!(!shows(&window, "The filters hide the row of this pod."));
}

/// Plan section 12.5: the keys are settings. A key that the settings give acts, and the
/// default key of that command does not.
#[test]
fn the_keys_of_the_settings_act_and_the_defaults_they_replace_do_not() {
    let shortcut = |text: &str| fresnel_core::commands::Shortcut::parse(text).expect("a shortcut");
    let (keys, problems) = Keys::with_choices(&[(CommandId::QuickSearch, Some(shortcut("Alt+Space")), ())]);
    assert!(problems.is_empty());
    let mut session = view(vec![searchable("ams-prod", StoreState::Live)]);
    session.config = Arc::new(Config { keys });
    let mut window = window(session);
    window.key_press_modifiers(Modifiers::COMMAND, Key::K);
    window.run();
    assert!(window.query_by_label("Search text").is_none(), "the default key does not act");
    window.key_press_modifiers(Modifiers::ALT, Key::Space);
    window.run();
    assert!(window.query_by_label("Search text").is_some(), "the key of the settings");
    let key = window.get_by_label("Quick search").value().expect("the key of the button");
    assert!(key == "⌥Space" || key == "Alt+Space", "the button tells the key of the settings: {key}");
}

/// Plan section 11.5: the menu of a row has the actions of the kind from the command table,
/// with their keys, and an action that this version does not run is off.
#[test]
fn the_menu_of_a_row_has_the_actions_of_its_kind_and_runs_one() {
    let mut window = window(view(vec![searchable("ams-prod", StoreState::Live)]));
    window.get_by_label("crash").click_secondary();
    window.run();
    for action in ["View manifest", "Copy name", "Logs", "Evict…", "Delete…"] {
        assert!(window.query_by_label_contains(action).is_some(), "the menu has {action}");
    }
    assert!(window.get_by_label_contains("Delete…").accesskit_node().is_disabled(), "a later milestone");
    window.get_by_label_contains("Copy kubectl command").click();
    window.run();
    let command = "kubectl --context ams-prod get pods --namespace scene crash --output yaml";
    assert!(shows(&window, &format!("Copied: {command}")));
    assert!(in_panel(&window, "crash"), "the menu opened the panel of its row");

    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::M);
    window.run();
    assert!(shows(&window, "managedFields"), "the key of View manifest opens the tab");
}

#[test]
fn the_key_of_the_filter_puts_the_focus_into_the_filter_of_the_page() {
    let mut window = window(view(vec![searchable("ams-prod", StoreState::Live)]));
    window.key_press_modifiers(Modifiers::COMMAND, Key::F);
    window.run();
    window.run();
    assert!(window.get_by_label("Filter pods").is_focused());
}

/// Plan section 11.1: one namespace filter for each view. The ConfigMaps keep the filter
/// that the Pods got, and a kind with no namespace ignores it.
#[test]
fn the_namespace_filter_of_the_top_bar_holds_for_each_page() {
    let clusters = vec![live_kind("ams-prod", &config_maps()), live_kind("fra-stage", &nodes())];
    let mut window = window(view(clusters));
    window.get_by_label("All namespaces").click();
    window.run();
    window.get_by_role_and_label(Role::CheckBox, "kube-system").click();
    until(&mut window, "the pods of one namespace", |window| shows(window, "4 of 50 pods"));
    window.key_press(Key::Escape);
    window.run();

    open_page(&mut window, "ConfigMaps");
    let none = "No ConfigMap passes the filters.";
    until(&mut window, "no ConfigMap of kube-system", |window| window.query_by_label_contains(none).is_some());
    assert!(shows(&window, "Namespace: kube-system"), "the top bar tells the filter");
    open_page(&mut window, "Nodes");
    until(&mut window, "each node", |window| !column(window, "Name").is_empty());
}

/// The Connections page has no filter and no object. The page of a kind that it keeps hidden
/// gives up its state in each frame, so a key of a page there leaves no trace for later.
#[test]
fn a_key_of_a_page_on_the_connections_page_leaves_no_trace_on_the_page() {
    let mut window = window(view(vec![searchable("ams-prod", StoreState::Live)]));
    open_page(&mut window, "Connections");
    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::C);
    window.key_press_modifiers(Modifiers::COMMAND, Key::F);
    window.run();
    open_page(&mut window, "Pods");
    until(&mut window, "the pods", |window| shows(window, "25 pods"));
    assert!(window.query_by_label_contains("No pod is open in the panel.").is_none());
    assert!(!window.get_by_label("Filter pods").is_focused());
}
