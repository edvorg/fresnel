//! Drags under stress: long moves, and new data and a table with no row during a drag.

use fresnel_core::app::Command;
use fresnel_core::state_file::TableLayout;

use crate::data::{live, pods};
use crate::fake::{shows, until, view, window};
use crate::pods::resize::Drag;
use crate::pods::scenes::pods_of;

fn save(widths: &[(&str, u32)]) -> Command {
    let widths = widths.iter().map(|(title, width)| ((*title).to_owned(), *width)).collect();
    Command::SaveTable { table: "Pods".to_owned(), layout: TableLayout { widths } }
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
