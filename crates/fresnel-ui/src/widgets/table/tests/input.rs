//! The sort, the clicks and the keys of a table.

use egui_kittest::kittest::Queryable as _;

use super::Bed;
use super::objects;
use crate::widgets::table::Act;

/// Plan section 11.2: each sort has a fixed last key.
#[test]
fn a_click_on_a_title_sorts_and_rows_with_equal_values_keep_one_sequence() {
    let mut bed = Bed::start(&[(1, 5), (2, 5), (3, 1), (4, 9)], false, false);
    bed.result();
    bed.window.step();
    let sort_by_value = |bed: &mut Bed| {
        bed.window.get_by_role_and_label(egui::accesskit::Role::Button, "Value").click();
        bed.window.step();
        bed.window.step();
        bed.result();
        bed.window.step();
        bed.shown()
    };
    assert_eq!(sort_by_value(&mut bed), ["object-3", "object-1", "object-2", "object-4"]);
    assert_eq!(sort_by_value(&mut bed), ["object-4", "object-1", "object-2", "object-3"], "1 stays before 2");

    bed.window.state_mut().filter = 100;
    bed.window.step();
    bed.result();
    bed.window.step();
    assert!(bed.window.query_by_label("no row passes the filter").is_some(), "the view tells why no row shows");
}

#[test]
fn clicks_and_keys_select_objects_and_escape_clears_the_selection() {
    let mut bed = Bed::start(&objects(1..=40), false, false);
    bed.result();
    bed.window.step();
    bed.click("object-2", egui::Modifiers::NONE);
    bed.click("object-4", egui::Modifiers::SHIFT);
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (3, Some("4")));
    bed.click("object-3", egui::Modifiers::COMMAND);
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (2, Some("3")), "Cmd removes one object");
    bed.click("object-1", egui::Modifiers::NONE);

    assert_eq!(bed.key(egui::Modifiers::NONE, egui::Key::ArrowDown), Some(Act::Cursor));
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (1, Some("2")));
    bed.key(egui::Modifiers::SHIFT, egui::Key::ArrowDown);
    bed.key(egui::Modifiers::SHIFT, egui::Key::ArrowDown);
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (3, Some("4")), "Shift makes a range from the anchor");
    bed.key(egui::Modifiers::SHIFT, egui::Key::ArrowUp);
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (2, Some("3")));

    // The last row is far below the window. The table scrolls to the row of the cursor.
    assert!(bed.window.query_by_label("object-40").is_none());
    bed.key(egui::Modifiers::NONE, egui::Key::End);
    bed.window.step();
    assert_eq!(bed.cursor().as_deref(), Some("40"));
    assert!(bed.window.query_by_label("object-40").is_some(), "the row of the cursor shows");
    assert_eq!(bed.key(egui::Modifiers::NONE, egui::Key::Home), Some(Act::Cursor));
    assert_eq!(bed.key(egui::Modifiers::COMMAND, egui::Key::A), None);
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (40, Some("1")), "Cmd+A selects each row");
    assert_eq!(bed.key(egui::Modifiers::NONE, egui::Key::Escape), Some(Act::Cleared));
    assert_eq!((bed.selected(), bed.cursor()), (0, None));
}
