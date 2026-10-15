//! Commands, keys and navigation (plan section 11.5): Back and Forward through the visited
//! objects, an object that a filter hides, the keys of the settings, the menu of a row and
//! the namespace filter of each view.

use std::sync::Arc;

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::ClusterRow;
use fresnel_core::commands::{CommandId, Keys};
use fresnel_core::config::Config;
use fresnel_core::kinds::Kind;
use fresnel_core::search::Source;
use fresnel_core::store::{Snapshot, StoreState};

use crate::data::{changed_object, held, live, pods, store};
use crate::fake::{Window, in_one_frame, open_page, press, shows, until, view, window};
use crate::kinds::in_panel;
use crate::kinds::scene::{config_maps, live as live_kind, nodes};
use crate::pods::column;
use crate::search::{self, search, searchable, secrets, until_hit, with_sources};

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
    // The panel reads the snapshot, and the count waits for the rows of the worker thread.
    until(&mut window, "the pod before the Secret on the page of the Pods", |window| {
        in_panel(window, "failed") && shows(window, "25 pods")
    });
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
    until(&mut window, "the second pod and the rows", |window| in_panel(window, "failed") && shows(window, "25 pods"));
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
    session.config = Arc::new(Config { keys, ..Config::default() });
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
    for action in ["View manifest", "Copy name", "Logs", "Terminal", "Evict…", "Delete…"] {
        assert!(window.query_by_label_contains(action).is_some(), "the menu has {action}");
    }
    assert!(window.get_by_label_contains("Terminal").accesskit_node().is_disabled(), "a later milestone");
    assert!(!window.get_by_label_contains("Logs").accesskit_node().is_disabled(), "the logs run in M9");
    assert!(!window.get_by_label_contains("Delete…").accesskit_node().is_disabled(), "a delete runs in M3");
    window.get_by_label_contains("Copy kubectl command").click();
    window.run();
    // The dialog has each command of the pod; a click copies one.
    window.get_by_label("Copy get").click();
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
    // The title of a column shows with the first rows of the worker thread.
    until(&mut window, "each node", |window| {
        window.query_by_role_and_label(Role::Button, "Name").is_some() && !column(window, "Name").is_empty()
    });
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

/// A slow frame gets each key that the user pressed since the frame before: two presses of
/// Back are two steps.
#[test]
fn two_presses_of_back_in_one_frame_are_two_steps() {
    let mut window = window(view(vec![searchable("ams-prod", StoreState::Live)]));
    for pod in ["crash", "failed", "done"] {
        click_row(&mut window, pod);
    }
    let back = press(Modifiers::COMMAND, Key::OpenBracket);
    in_one_frame(&mut window, [back.clone(), back].concat());
    until(&mut window, "a step back", |window| !in_panel(window, "done"));
    assert!(in_panel(&window, "crash"), "two steps back");
}

/// A cluster whose Secrets page waits for its list: the store of the metadata has each
/// Secret, and the store of the full objects has the Secret `other` only, until `listed`.
fn waiting_secrets(listed: bool) -> ClusterRow {
    let other = changed_object(&*secrets()[0], |value| {
        value["metadata"]["name"] = "other".into();
        value["metadata"]["uid"] = "uid-other".into();
    });
    let metadata = Source::metadata(Kind::Secret, held(store(StoreState::Live, &search::metadata(&secrets()))));
    let mut row = with_sources(live("ams-prod", &pods()), vec![metadata]);
    let full = if listed {
        Snapshot { generation: 2, ..store(StoreState::Live, &[secrets()[0].clone(), other]) }
    } else {
        Snapshot { listed: false, ..store(StoreState::Listing { loaded: 1 }, &[other]) }
    };
    if let Some(data) = row.data.as_mut() {
        data.objects = data.objects.clone().with(held(full));
    }
    row
}

/// The object of a link waits for the list of its store, with a note. A row that the user
/// clicks in that time ends the wait, so the list that arrives later does not take the
/// panel from it.
#[test]
fn a_link_that_waits_for_its_store_has_a_note_and_a_later_click_ends_the_wait() {
    let mut window = window(view(vec![waiting_secrets(false)]));
    search(&mut window, "web-secret");
    until_hit(&mut window, "Secret kinds/web-secret of ams-prod");
    window.key_press(Key::Enter);
    let note = "The Secret web-secret of ams-prod opens when its store has a complete list.";
    until(&mut window, "the note of the link and the row of the worker thread", |window| {
        shows(window, note) && shows(window, "other")
    });
    click_row(&mut window, "other");
    until(&mut window, "the panel of the row", |window| in_panel(window, "other"));
    assert!(!shows(&window, note), "the click ended the wait");
    window.state_mut().view = view(vec![waiting_secrets(true)]);
    until(&mut window, "the complete list", |window| shows(window, "2 Secrets"));
    assert!(in_panel(&window, "other") && !in_panel(&window, "web-secret"), "the panel keeps the row of the click");
}
