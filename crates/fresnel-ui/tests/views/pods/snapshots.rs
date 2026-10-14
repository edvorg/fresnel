//! PNG files of the Pods view in the dark theme, for a look by a person.

use egui_kittest::kittest::Queryable as _;
use fresnel_core::store::{StoreState, Wait};

use crate::data::{cluster, empty_store, live, many_pods, pods};
use crate::fake::{SECOND, Window, connected, live_status, now, open_tab, until, view, window_with};
use crate::pods::resize::drag;
use crate::pods::scenes::{pods_of, session};
use crate::snapshots::{SCALE, save};

/// Saves the frame with no pointer arrow: the harness draws one at the last click.
fn save_clean(window: &mut Window, name: &str) {
    window.remove_cursor();
    window.run();
    save(window, name);
}

#[test]
fn the_pods_view_with_old_rows_and_the_notes_of_three_clusters() {
    let mut window = window_with(session(), egui::Theme::Dark, SCALE);
    save(&mut window, "pods");
}

#[test]
fn the_pods_of_two_live_clusters_above_the_connections_tab() {
    let clusters = vec![live("ams-prod", &pods()), live("fra-stage", &pods_of("fra"))];
    let mut window = window_with(view(clusters), egui::Theme::Dark, SCALE);
    open_tab(&mut window, "Connections");
    save_clean(&mut window, "pods-live");
}

/// The edges of three columns after a drag: Name and Status are wider, Node is narrower.
#[test]
fn the_pods_of_two_clusters_after_a_drag_of_three_column_edges() {
    let clusters = vec![live("ams-prod", &pods()), live("fra-stage", &pods_of("fra"))];
    let mut window = window_with(view(clusters), egui::Theme::Dark, SCALE);
    drag(&mut window, "Name", 70.0);
    drag(&mut window, "Status", 50.0);
    drag(&mut window, "Node", -70.0);
    save_clean(&mut window, "pods-resized");
}

#[test]
fn the_detail_panel_of_a_pod_with_its_events() {
    let mut window = window_with(session(), egui::Theme::Dark, SCALE);
    window.get_by_label("crash").click();
    window.run();
    save_clean(&mut window, "pods-detail");
}

#[test]
fn the_problems_filter_and_some_selected_rows() {
    let mut window = window_with(session(), egui::Theme::Dark, SCALE);
    window.get_by_label("Problems").click();
    until(&mut window, "the pods that are not healthy", |window| {
        window.query_all_by_label_contains(" of 37 pods").next().is_some()
    });
    window.get_by_label("crash").click();
    window.get_by_label("fails-l8dvz").click_modifiers(egui::Modifiers::SHIFT);
    window.run();
    save_clean(&mut window, "pods-problems");
}

#[test]
fn the_first_lists_of_three_clusters_with_no_row_yet() {
    let listing = |loaded| empty_store(StoreState::Listing { loaded }, now() - SECOND);
    let clusters = vec![
        cluster("ams-prod", listing(0)),
        cluster("fra-stage", empty_store(StoreState::Waiting(Wait::ListSlot), now() - SECOND)),
        connected("sgp-prod", live_status()),
    ];
    let mut window = window_with(view(clusters), egui::Theme::Dark, SCALE);
    save(&mut window, "pods-loading");
}

#[test]
fn ten_thousand_pods_of_one_cluster() {
    let many = many_pods(10_000);
    let mut window = window_with(view(vec![live("ams-prod", &many)]), egui::Theme::Dark, SCALE);
    save(&mut window, "pods-10000");
}
