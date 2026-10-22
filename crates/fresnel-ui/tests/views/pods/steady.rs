//! The rows stay under the pointer when a note above the table goes away. egui gives a
//! press to the row that the frame before the press had at the pointer, and a person aims
//! at the rows of a frame some time before the press. The note of a store that waits for
//! the start of its watch goes away when the watch opens: with no hold, the rows moved up
//! by the height of the note, and the click opened the row below the row of the person.

use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::store::{StoreState, Wait};

use crate::data::{cluster, live, pods, store};
use crate::fake::{Window, shows, view, window};

const NOTE: &str = "Pods: waits for the start of its watch. Its rows are old since 07:59:12.";

/// The row of a pod: the label with its name nearest to the left, not the heading of the panel.
fn row(window: &Window, name: &str) -> Rect {
    let labels = window.query_all_by_label(name).map(|node| node.rect());
    labels.min_by(|a, b| a.left().total_cmp(&b.left())).expect("the row")
}

/// A click of a person at this place: the move, the press and the release, each in a frame
/// of its own, as a window gets them and as the harness gives them.
fn click_at(window: &mut Window, place: Pos2) {
    let button = |pressed| Event::PointerButton {
        pos: place,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    for event in [Event::PointerMoved(place), button(true), button(false)] {
        window.event(event);
    }
    window.run();
}

#[test]
fn a_note_that_goes_away_in_the_frame_of_a_click_moves_no_row_under_the_pointer() {
    let mut window = window(view(vec![cluster("ams-prod", store(StoreState::Waiting(Wait::Stream), &pods()))]));
    assert!(shows(&window, NOTE));
    let place = row(&window, "db-0").center();

    // The watch opens, so the frame of the move of the click has no note.
    window.state_mut().view = view(vec![live("ams-prod", &pods())]);
    click_at(&mut window, place);
    assert!(!shows(&window, NOTE), "the store is live");
    let pod_index = |number: u32| shows(&window, &format!("apps.kubernetes.io/pod-index={number}"));
    assert!(pod_index(0) && !pod_index(1), "the click opens db-0, the row at the pointer, and not db-1");
    assert!((row(&window, "db-0").center().y - place.y).abs() < 0.5, "the row stays under the pointer");

    // The pointer leaves the table, and the rows take the place of the note.
    let sidebar = window.get_by_role_and_label(Role::RadioButton, "Pods").rect().center();
    window.event(Event::PointerMoved(sidebar));
    window.run();
    assert!(row(&window, "db-0").center().y < place.y - 10.0, "the rows move up");
}
