//! The dialog of the quick search: its keys, its filters, the marks of its hits and the
//! commands of the command table.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::ClusterRow;
use fresnel_core::cluster::LinkState;
use fresnel_core::store::StoreState;

use super::{search, searchable, until_hit, with_sources};
use crate::data::{cluster, in_state, pods, store};
use crate::fake::{Window, error, now, open_page, row, shows, until, view, window};
use crate::kinds::in_panel;

fn is_open(window: &Window) -> bool {
    window.query_by_label("Search text").is_some()
}

/// A click as a mouse gives it: the pointer moves onto the widget in one frame, and the
/// button goes down and up in the next one. Over the list of the dialog, a widget gets a
/// click only after the pointer was on it.
fn press(window: &mut Window, node: impl for<'a> Fn(&'a Window) -> egui_kittest::Node<'a>) {
    node(window).hover();
    window.run();
    node(window).click();
    window.run();
}

#[test]
fn a_key_or_the_button_opens_the_search_on_each_page_and_escape_closes_it() {
    let mut window = window(view(vec![searchable("ams-prod", StoreState::Live)]));
    for page in ["Pods", "Secrets", "Connections"] {
        open_page(&mut window, page);
        window.key_press_modifiers(Modifiers::COMMAND, Key::K);
        window.run();
        assert!(is_open(&window), "{page}: the key opens the search");
        window.key_press(Key::Escape);
        window.run();
        assert!(!is_open(&window), "{page}: Escape closes it");
    }
    window.get_by_label("Quick search").click();
    window.run();
    assert!(is_open(&window), "the button of the top bar");
    assert!(window.get_by_label("Search text").is_focused());
}

/// The dialog opens with the text of the last search, selected: the first key of a new
/// search replaces it.
#[test]
fn the_text_of_the_last_search_is_selected_and_a_key_replaces_it() {
    let mut window = window(view(vec![searchable("ams-prod", StoreState::Live)]));
    search(&mut window, "web");
    window.key_press(Key::Escape);
    window.run();
    assert!(!is_open(&window));

    search(&mut window, "c");
    assert_eq!(window.get_by_label("Search text").value().as_deref(), Some("c"));
    until_hit(&mut window, "Pod scene/crash of ams-prod");
}

/// Design rule 4: a hit of a store that is not live has the mark of its rows.
#[test]
fn a_hit_of_a_store_that_is_not_live_has_the_mark_old_since() {
    let backoff = StoreState::Backoff { failures: 2, reason: error("reset"), until: now() };
    let old = with_sources(cluster("ams-prod", in_state(&store(StoreState::Live, &pods()), backoff)), Vec::new());
    let live = with_sources(cluster("fra-stage", store(StoreState::Live, &pods())), Vec::new());
    let mut window = window(view(vec![old, live]));
    search(&mut window, "init-crash");
    until_hit(&mut window, "Pod scene/init-crash of ams-prod, old since 07:59:12");
    assert!(window.query_by_label("Pod scene/init-crash of fra-stage").is_some(), "a live store has no mark");
}

#[test]
fn the_cluster_and_the_kind_are_filters_of_the_search() {
    let clusters = vec![searchable("ams-prod", StoreState::Live), searchable("fra-stage", StoreState::Live)];
    let mut window = window(view(clusters));
    search(&mut window, "web");
    until_hit(&mut window, "Secret kinds/web-secret of fra-stage");
    assert!(window.query_by_label("Secret kinds/web-secret of ams-prod").is_some());

    press(&mut window, |window| window.get_by_label("All clusters"));
    // The switch of the menu, not the switch of the cluster in the top bar.
    press(&mut window, |window| {
        window.get_all_by_role_and_label(Role::CheckBox, "fra-stage").last().expect("a switch")
    });
    until(&mut window, "the hits of fra-stage only", |window| {
        window.query_by_label("Secret kinds/web-secret of ams-prod").is_none()
    });
    assert!(window.query_by_label("Secret kinds/web-secret of fra-stage").is_some());
    assert!(shows(&window, "Cluster: fra-stage"), "the menu tells the filter");
    window.key_press(Key::Escape);
    window.run();

    press(&mut window, |window| window.get_by_label("All kinds"));
    press(&mut window, |window| window.get_by_role_and_label(Role::CheckBox, "Pods"));
    until(&mut window, "no Secret", |window| window.query_by_label("Secret kinds/web-secret of fra-stage").is_none());
    assert!(shows(&window, "Kind: Pods"));
}

#[test]
fn the_arrows_move_the_cursor_and_enter_opens_its_hit() {
    let mut window = window(view(vec![searchable("ams-prod", StoreState::Live)]));
    search(&mut window, "crash");
    until_hit(&mut window, "Pod scene/init-crash of ams-prod");
    // The best hit first: the name itself, then a name that has the word in it.
    window.key_press(Key::ArrowDown);
    window.key_press(Key::Enter);
    window.run();
    assert!(!is_open(&window));
    until(&mut window, "the panel of the second hit", |window| in_panel(window, "init-crash"));
}

/// A selected cluster with no stores is a note: the search did not read it.
#[test]
fn a_cluster_with_no_stores_is_a_note_with_its_reason() {
    let failed: ClusterRow = row("nyc-prod", LinkState::Failed(error("the kubeconfig entry has no server")));
    let mut window = window(view(vec![searchable("ams-prod", StoreState::Live), failed]));
    search(&mut window, "web");
    until_hit(&mut window, "Secret kinds/web-secret of ams-prod");
    assert!(shows(&window, "Not read: nyc-prod: no store: the kubeconfig entry has no server"));
}

/// Plan section 11.5: the quick search lists the commands of the command table.
#[test]
fn a_command_of_the_table_is_a_row_and_enter_runs_it() {
    let mut window = window(view(vec![searchable("ams-prod", StoreState::Live)]));
    window.get_by_label("crash").click();
    window.run();
    search(&mut window, "copy name");
    until(&mut window, "the command", |window| window.query_by_label("Command Copy name").is_some());
    window.key_press(Key::Enter);
    window.run();
    assert!(!is_open(&window));
    assert!(shows(&window, "Copied the name crash."), "the command acted on the object of the panel");
}
