//! The choice of the columns of a table (plan section 11.2) and its place in the saved layout.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::Command;
use fresnel_core::state_file::TableLayout;
use k8s_openapi::api::apps::v1::Deployment;

use super::scene::{deployments, live};
use super::weakness::page;
use crate::fake::{Window, until, view};
use crate::pods::column;

fn has_title(window: &Window, title: &str) -> bool {
    window.query_by_role_and_label(Role::Button, title).is_some()
}

fn choose(window: &mut Window, title: &str) {
    window.get_by_role_and_label(Role::CheckBox, title).click();
    window.run();
}

#[test]
fn the_user_shows_and_hides_columns_and_the_state_file_keeps_the_choice() {
    let mut window = page::<Deployment>(vec![live("ams-prod", &deployments())], 4);
    assert!(has_title(&window, "Images") && !has_title(&window, "Labels"), "the first choice of the kind");
    window.get_by_label("Columns").click();
    window.run();
    assert!(window.query_by_role_and_label(Role::CheckBox, "Name").is_none(), "the name always shows");
    choose(&mut window, "Images");
    choose(&mut window, "Labels");
    window.key_press(egui::Key::Escape);
    until(&mut window, "the new columns", |window| !has_title(window, "Images") && has_title(window, "Labels"));
    let labels = column(&window, "Labels");
    assert!(labels.iter().any(|labels| labels.starts_with("k8s-app=kube-dns, ")), "{labels:?}");

    let columns = BTreeMap::from([("Images".to_owned(), false), ("Labels".to_owned(), true)]);
    let layout = TableLayout { columns, ..TableLayout::default() };
    let saved = Command::SaveTable { table: "Deployments".to_owned(), layout: layout.clone() };
    assert_eq!(window.state().sent.last(), Some(&saved));

    // The next run of the app reads the layout, and the table shows the same columns at once.
    let mut next = view(vec![live("ams-prod", &deployments())]);
    next.tables = Some(Arc::new(BTreeMap::from([("Deployments".to_owned(), layout)])));
    let mut window = crate::fake::window(next);
    crate::fake::open_page(&mut window, "Deployments");
    until(&mut window, "the saved columns", |window| has_title(window, "Labels"));
    assert!(!has_title(&window, "Images"));
    let commands = window.state().sent.iter().filter(|command| matches!(command, Command::SaveTable { .. })).count();
    assert_eq!(commands, 0, "a saved layout is no change of the user");
}

/// A hidden column cannot show the arrow of its sort. The rows of a sort column that the
/// user hides go back to the sort by the name, and the name shows the arrow.
#[test]
fn the_rows_of_a_sort_column_that_the_user_hides_go_back_to_the_sort_by_the_name() {
    let mut window = page::<Deployment>(vec![live("ams-prod", &deployments())], 4);
    let by_name = ["broken", "coredns", "local-path-provisioner", "web"];
    assert_eq!(column(&window, "Name"), by_name);
    window.get_by_role_and_label(Role::Button, "Images").click();
    let by_image = ["local-path-provisioner", "coredns", "web", "broken"];
    until(&mut window, "the sort by the images", |window| column(window, "Name") == by_image);
    window.get_by_label("Columns").click();
    window.run();
    choose(&mut window, "Images");
    window.key_press(egui::Key::Escape);
    until(&mut window, "the column goes", |window| !has_title(window, "Images"));
    until(&mut window, "the sort by the name", |window| column(window, "Name") == by_name);
}
