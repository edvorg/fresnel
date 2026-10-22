//! A press, its click and an open menu keep the object of the row at the press. The worker
//! can sort or change the rows between a press and its release: with the number of a row as
//! the identity of its widget, the click and the action of the menu went to the object that
//! came to that row.

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable as _;

use super::{Bed, ENTRY, objects};
use crate::widgets::table::Act;
use crate::widgets::table::model::Sort;

fn button(place: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: place, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

#[test]
fn a_click_selects_the_object_of_the_press_when_new_rows_come_before_the_release() {
    let mut bed = Bed::start(&objects(1..=3), false, false);
    bed.result();
    bed.window.step();
    let place = bed.window.get_by_label("object-1").rect().center();
    bed.window.event(Event::PointerMoved(place));
    bed.window.event(button(place, true));
    bed.window.step();

    // A new object comes before the release, and its row is the row of the press. A press
    // that lasts more than 0.8 s is no click, and each frame of the harness takes 0.25 s.
    bed.set(&[vec![(0, 0)], objects(1..=3)].concat());
    bed.result();
    bed.window.step();
    assert_eq!(bed.shown(), ["object-0", "object-1", "object-2", "object-3"]);
    bed.window.event(button(place, false));
    bed.window.step();
    assert_eq!(bed.window.state().acted, Some(Act::Cursor));
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (1, Some("1")), "the object of the press, not object-0");

    // A row has a widget in each part of `egui_table`: here the part that scrolls to the side.
    assert_eq!(bed.click("20", Modifiers::NONE), Some(Act::Cursor));
    assert_eq!(bed.cursor().as_deref(), Some("2"));
}

#[test]
fn the_menu_of_a_row_acts_on_its_object_after_a_sort_while_it_is_open() {
    let mut bed = Bed::start(&objects(1..=3), false, false);
    bed.result();
    bed.window.step();
    bed.window.get_by_label("object-1").click_secondary();
    bed.window.step();
    assert!(bed.window.query_by_label(ENTRY).is_some(), "the menu of object-1 is open");

    // The worker sorts the rows while the menu is open: object-3 comes to the row of object-1.
    bed.window.state_mut().table.supply.set_sort(Sort { column: 0, descending: true });
    bed.window.step();
    bed.result();
    bed.window.step();
    assert_eq!(bed.shown(), ["object-3", "object-2", "object-1"]);
    bed.window.get_by_label(ENTRY).click();
    bed.window.step();
    let chosen = bed.window.state().chosen.as_ref().map(|id| id.uid.to_string());
    assert_eq!(chosen.as_deref(), Some("1"), "the menu acts on the object that it was opened for");
}
