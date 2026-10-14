//! The widths of the Pods table in the state file (plan section 12.5). A drag of the user
//! gives one command after its end, and the saved widths show when the core read the file.

use std::collections::BTreeMap;
use std::sync::Arc;

use fresnel_core::app::{Command, CoreView};
use fresnel_core::state_file::TableLayout;

use crate::data::{live, pods};
use crate::fake::{until, view, window};
use crate::pods::resize::{drag, width};

fn layout(widths: &[(&str, u32)]) -> TableLayout {
    TableLayout { widths: widths.iter().map(|(title, width)| ((*title).to_owned(), *width)).collect() }
}

fn save(widths: &[(&str, u32)]) -> Command {
    Command::SaveTable { table: "Pods".to_owned(), layout: layout(widths) }
}

/// The tables of the state file with this layout of the Pods table.
fn saved_pods(widths: &[(&str, u32)]) -> Option<Arc<BTreeMap<String, TableLayout>>> {
    Some(Arc::new(BTreeMap::from([("Pods".to_owned(), layout(widths))])))
}

/// The view of a core that did not read the state file yet.
fn before_the_read() -> CoreView {
    CoreView { tables: None, ..view(vec![live("ams-prod", &pods())]) }
}

#[test]
fn a_drag_saves_the_widths_one_time_after_its_end() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    drag(&mut window, "Name", 70.0);
    assert_eq!(window.state().sent, [save(&[("Name", 290)])], "one command, after the end of the drag");
    drag(&mut window, "Ready", 44.0);
    // A width back at the width of the table leaves the layout.
    drag(&mut window, "Name", -70.0);
    let expected = [save(&[("Name", 290)]), save(&[("Name", 290), ("Ready", 100)]), save(&[("Ready", 100)])];
    assert_eq!(window.state().sent, expected);
}

#[test]
fn the_saved_widths_show_when_the_core_read_the_state_file() {
    let mut window = window(before_the_read());
    assert_eq!((width(&window, "Name"), width(&window, "Ready")), (220.0, 56.0), "the widths of the table");

    window.state_mut().view.tables = saved_pods(&[("Name", 300), ("Ready", 80), ("Gone", 90)]);
    until(&mut window, "the saved widths", |window| width(window, "Name") == 300.0);
    assert_eq!(width(&window, "Ready"), 80.0);
    assert_eq!(window.state().sent, [], "a saved layout is no change of the user");

    // The table takes the saved layout one time: the next ones are its own.
    window.state_mut().view.tables = saved_pods(&[]);
    window.run();
    assert_eq!(width(&window, "Name"), 300.0);
}

#[test]
fn a_width_that_the_user_changed_before_the_read_of_the_file_stays_and_is_saved() {
    let mut window = window(before_the_read());
    drag(&mut window, "Name", 70.0);
    assert_eq!(window.state().sent, [], "the core does not write a file that it did not read");

    window.state_mut().view.tables = saved_pods(&[("Ready", 80)]);
    until(&mut window, "the command of the drag", |window| !window.state().sent.is_empty());
    assert_eq!(window.state().sent, [save(&[("Name", 290)])]);
    assert_eq!((width(&window, "Name"), width(&window, "Ready")), (290.0, 56.0), "the widths of the user");
}

/// A layout that the earlier version of the table saved loads, with a title that the
/// table no longer has, and a drag with the handles of now keeps each other saved width.
#[test]
fn a_layout_of_the_earlier_version_loads_and_a_drag_keeps_its_other_widths() {
    let mut window = window(before_the_read());
    window.state_mut().view.tables = saved_pods(&[("Name", 300), ("Ready", 80), ("Gone", 90)]);
    until(&mut window, "the saved widths", |window| width(window, "Name") == 300.0);
    drag(&mut window, "Namespace", 30.0);
    assert_eq!(width(&window, "Namespace"), 150.0);
    assert_eq!(window.state().sent, [save(&[("Name", 300), ("Namespace", 150), ("Ready", 80)])]);
}
