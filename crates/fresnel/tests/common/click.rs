//! A click on a node of the window whose place can change from one frame to the next, as
//! the row of an object or a link of an overview.
//!
//! kittest gives each event of a click a frame of its own, and egui gives a press to the
//! widget under the pointer in the frame before the press: the frame of the move. A click at
//! the place of the last frame thus misses when the frame of the move puts the node at an
//! other place, as a note above a table that comes in that frame. So the press comes only
//! where the frame of the move has the node, and the move comes again until a frame has it.
//! A press that egui gave to the node must open what the click opens: a second press would
//! hide a click that the app lost, and a person would lose that click too.

use std::time::{Duration, Instant};

use eframe::egui;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

use crate::window::{LIMIT, Window};

/// Clicks the first node with this role and label, and waits until the window meets `done`,
/// such as the panel of a row. Gives the time of the wait.
pub(crate) fn click_until(window: &mut Window, role: Role, label: &str, done: impl Fn(&Window) -> bool) -> Duration {
    let journal = |window: &Window| format!("{:#?}", window.state().core.view().journal.entries);
    click_within(window, (role, label), LIMIT, done, journal)
}

/// Moves the pointer to the first node with this role and label until the frame of the move
/// has the node at the pointer, presses there one time, and draws frames until the window
/// meets `done`, for `limit` at most. `tell` gives the state of the window for a failure.
pub(crate) fn click_within<'a, S>(
    window: &mut Harness<'a, S>,
    (role, label): (Role, &str),
    limit: Duration,
    done: impl Fn(&Harness<'a, S>) -> bool,
    tell: impl Fn(&Harness<'a, S>) -> String,
) -> Duration {
    let find = |window: &Harness<'a, S>| window.query_all_by_role_and_label(role, label).next().map(|node| node.rect());
    let started = Instant::now();
    let mut pressed = false;
    loop {
        window.run();
        if done(window) {
            return started.elapsed();
        }
        let what = if pressed { "after a press on it" } else { "and no frame of a move had it at the pointer" };
        assert!(started.elapsed() < limit, "in time: what a click of {label} opens, {what}: {}", tell(window));
        if !pressed && let Some(rect) = find(window) {
            pressed = press_in_place(window, rect.center(), find);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Moves the pointer to `place`, and presses and releases there when the frame of the move
/// has the node of `find` under the pointer. Gives true when it pressed.
pub(crate) fn press_in_place<'a, S>(
    window: &mut Harness<'a, S>,
    place: egui::Pos2,
    find: impl Fn(&Harness<'a, S>) -> Option<egui::Rect>,
) -> bool {
    window.event(egui::Event::PointerMoved(place));
    window.step();
    if !find(window).is_some_and(|rect| rect.contains(place)) {
        return false;
    }
    let button = |pressed| egui::Event::PointerButton {
        pos: place,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    window.event(button(true));
    window.event(button(false));
    window.step();
    true
}
