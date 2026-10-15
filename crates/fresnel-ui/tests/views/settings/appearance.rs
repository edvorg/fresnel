//! The settings that act on the window at once: the theme, the time zone and the form of a
//! time of day, the height of a row, the alias and the read-only mark of a cluster, the
//! cluster groups, and the default and favorite namespaces.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::Command;
use fresnel_core::cluster::ClusterId;
use fresnel_core::journal::Level;

use super::with_config;
use crate::data::{live, pods};
use crate::fake::{Window, journal, open_tab, shows, until, view, window};

fn dark(window: &Window) -> bool {
    window.ctx.theme() == egui::Theme::Dark
}

#[test]
fn the_theme_of_the_file_acts_at_once_and_a_file_with_no_theme_keeps_the_theme_of_the_system() {
    let clusters = || vec![live("ams-prod", &pods())];
    let mut window = window(with_config(view(clusters()), "version = 1\n[appearance]\ntheme = \"light\"\n"));
    window.run();
    assert!(!dark(&window), "the light theme of the file in a dark window");
    window.state_mut().view = with_config(view(clusters()), "version = 1\n[appearance]\ntheme = \"dark\"\n");
    window.run();
    assert!(dark(&window));
    window.state_mut().view = with_config(view(clusters()), "version = 1\n");
    window.run();
    assert_eq!(window.ctx.options(|options| options.theme_preference), egui::ThemePreference::System);
}

#[test]
fn the_clock_has_the_time_zone_and_the_form_of_the_file() {
    let text = "version = 1\n[appearance]\ntime_zone = \"America/New_York\"\ntime_format = \"12h\"\n";
    let mut session = with_config(view(Vec::new()), text);
    session.journal = journal(&[(Level::Info, "app", "read 1 context from 1 kubeconfig file")], 0);
    let mut window = window(session);
    open_tab(&mut window, "Journal");
    // 08:00:00 UTC is 03:00:00 in New York, and the entry is one second before now.
    assert!(shows(&window, "02:59:59 AM"), "the time of the entry in the zone of the file");
}

#[test]
fn a_cluster_shows_its_alias_and_its_read_only_mark_and_a_group_selects_its_clusters() {
    let text =
        "version = 1\n[clusters.ams-prod]\nalias = \"AMS\"\nread_only = true\n[groups]\neurope = [\"fra-prod\"]\n";
    let mut window = window(with_config(view(vec![live("ams-prod", &pods()), live("fra-prod", &pods())]), text));
    assert!(window.query_by_role_and_label(Role::CheckBox, "AMS (read-only)").is_some(), "the chip of the top bar");
    assert!(shows(&window, "AMS: live") && shows(&window, "fra-prod: live"), "the status bar");
    window.get_by_label("Select the group europe").click();
    window.run();
    assert_eq!(
        window.state().sent,
        [Command::Select { cluster: ClusterId::new("ams-prod"), selected: false }],
        "fra-prod is selected already, and ams-prod is not in the group"
    );
}

#[test]
fn the_filter_starts_with_the_default_namespace_and_the_menu_lists_the_favorites_first() {
    let text =
        "version = 1\n[clusters.ams-prod]\ndefault_namespace = \"scene\"\nfavorite_namespaces = [\"kube-system\"]\n";
    let mut window = window(with_config(view(vec![live("ams-prod", &pods())]), text));
    assert!(shows(&window, "Namespace: scene"), "the filter of the first frames");
    window.get_by_label("Namespace: scene").click();
    window.run();
    let entries: Vec<String> =
        window.query_all_by_role(Role::CheckBox).filter_map(|node| node.accesskit_node().label()).collect();
    let favorite = entries.iter().position(|entry| entry == "★ kube-system");
    let scene = entries.iter().position(|entry| entry == "scene");
    assert!(favorite.zip(scene).is_some_and(|(favorite, scene)| favorite < scene), "{entries:?}");
}

/// The smallest distance between the rows of the pods on the screen.
fn row_step(text: &str) -> f32 {
    let mut window = window(with_config(view(vec![live("ams-prod", &pods())]), text));
    let names: Vec<String> = pods().iter().filter_map(|pod| pod.metadata.name.clone()).collect();
    until(&mut window, "the rows", |window| names.iter().filter(|name| shows(window, name)).count() >= 3);
    let mut tops: Vec<f32> =
        names.iter().filter_map(|name| window.query_all_by_label(name).next().map(|node| node.rect().top())).collect();
    tops.sort_by(f32::total_cmp);
    let steps = tops.windows(2).filter_map(|pair| match pair {
        [above, below] => Some(below - above),
        _ => None,
    });
    steps.fold(f32::MAX, f32::min)
}

#[test]
fn the_height_of_a_row_of_the_file_reaches_the_tables() {
    assert_eq!(row_step("version = 1\n"), 24.0, "the default");
    assert_eq!(row_step("version = 1\n[table]\nrow_height = 36\n"), 36.0);
}
