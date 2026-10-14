//! Drags under stress: long moves, new data and a table with no row during a drag, and a
//! first column wider than the table.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::Rangef;
use fresnel_core::app::{Command, CoreView};
use fresnel_core::state_file::TableLayout;

use crate::data::{live, pods};
use crate::fake::{Window, shows, until, view, window};
use crate::pods::resize::{Drag, TITLES, drag, span_of, spans, table_right, width};
use crate::pods::scenes::pods_of;

fn save(widths: &[(&str, u32)]) -> Command {
    let widths = widths.iter().map(|(title, width)| ((*title).to_owned(), *width)).collect();
    Command::SaveTable { table: "Pods".to_owned(), layout: TableLayout { widths } }
}

/// The span of a column whose title shows.
fn span(window: &Window, title: &str) -> Rangef {
    span_of(&spans(window), title).unwrap_or_else(|| panic!("the title {title} shows"))
}

/// The pointer goes far in one frame: past the narrowest width, to the left of the column,
/// and back. The edge follows it inside the limits of a width.
#[test]
fn one_drag_with_long_moves_past_the_narrowest_width_and_back() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    let drag = Drag::start(&mut window, "Namespace");
    for by in [300.0, -400.0, -100.0, 30.0] {
        drag.move_to(&mut window, by);
    }
    drag.end(&mut window, 30.0);
    assert_eq!(window.state().sent, [save(&[("Namespace", 150)])], "one command, after the end of the drag");
}

/// New rows during a drag, and rows that go away and come back: the edge stays at the
/// pointer, and the drag gives one command after its end.
#[test]
fn new_data_and_a_table_with_no_row_during_a_drag() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    let drag = Drag::start(&mut window, "Cluster");
    drag.move_to(&mut window, 40.0);
    window.state_mut().view = view(vec![live("ams-prod", &pods()), live("fra-stage", &pods_of("fra"))]);
    until(&mut window, "the rows of two clusters", |window| shows(window, "31 pods"));
    drag.move_to(&mut window, 80.0);

    window.state_mut().view = view(vec![live("ams-prod", &[])]);
    until(&mut window, "a table with no row", |window| shows(window, "0 pods"));
    window.state_mut().view = view(vec![live("ams-prod", &pods())]);
    until(&mut window, "the rows again", |window| shows(window, "25 pods"));
    drag.move_to(&mut window, -30.0);
    drag.end(&mut window, -30.0);
    assert_eq!(window.state().sent, [save(&[("Cluster", 90)])]);
}

/// A window whose core read a state file with this width for each column of the Pods table.
fn with_each_column(saved: u32, size: egui::Vec2) -> Window {
    let mut window = window(CoreView { tables: None, ..view(vec![live("ams-prod", &pods())]) });
    window.set_size(size);
    let widths = TITLES.iter().map(|title| ((*title).to_owned(), saved)).collect();
    window.state_mut().view.tables = Some(Arc::new(BTreeMap::from([("Pods".to_owned(), TableLayout { widths })])));
    until(&mut window, "the saved widths", |window| width(window, "Namespace") == saved as f32);
    window
}

/// The first column does not scroll to the side. Wider than the table, it would hide its
/// own edge and each other column. It stays inside the table, and a drag of its edge stops
/// there.
#[test]
fn a_first_column_wider_than_the_table_keeps_its_edge_and_the_other_columns_on_the_screen() {
    let mut window = with_each_column(2000, egui::vec2(900.0, 700.0));
    let right = table_right(&window);
    let name = span(&window, "Name");
    assert!(name.max < right, "the edge of Name at {} shows, left of {right}", name.max);
    assert!(span(&window, "Namespace").min < right, "the next column shows");

    let widest = name.span();
    drag(&mut window, "Name", -60.0);
    let drag = Drag::start(&mut window, "Name").with_widest(widest);
    drag.move_to(&mut window, 100.0);
    drag.end(&mut window, 100.0);
    let saved = |name: f32| {
        let mut widths: BTreeMap<String, u32> = TITLES.iter().map(|title| ((*title).to_owned(), 2000)).collect();
        widths.insert("Name".to_owned(), name.round() as u32);
        Command::SaveTable { table: "Pods".to_owned(), layout: TableLayout { widths } }
    };
    assert_eq!(window.state().sent, [saved(widest - 60.0), saved(widest)], "the widths on the screen");
}

/// A window that is narrower for a time shows less of the first column, and changes no
/// width of the user.
#[test]
fn a_narrow_window_for_a_time_changes_no_width() {
    let mut window = with_each_column(2000, egui::vec2(2600.0, 700.0));
    assert_eq!(width(&window, "Name"), 2000.0);
    window.set_size(egui::vec2(900.0, 700.0));
    window.run();
    assert!(span(&window, "Name").max < table_right(&window), "the edge of Name shows");
    window.set_size(egui::vec2(2600.0, 700.0));
    window.run();
    assert_eq!(width(&window, "Name"), 2000.0, "the saved width");
    assert_eq!(window.state().sent, [], "a window size is no change of the user");
}
