//! A click on a node of the window whose place can change from one frame to the next, as
//! the row of an object or a link of an overview.
//!
//! kittest gives each event of a click a frame of its own, and egui gives a press to the
//! widget under the pointer in the frame before the press: the frame of the move. A click at
//! the place of the last frame thus misses when the frame of the move puts the node at an
//! other place. A note above a table does that when it goes away: a store that has its list
//! and waits for its watch has one, and the first frames on a slow runner can show the rows of
//! that store with its note. In a failure on Windows, a press went below the only row.

use std::time::{Duration, Instant};

use eframe::egui;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

use crate::window::{LIMIT, Window};

/// Clicks the first node with this role and label until the window meets `done`, such as
/// the panel of a row. A click that opened nothing comes again. Gives the time of the wait.
pub(crate) fn click_until(window: &mut Window, role: Role, label: &str, done: impl Fn(&Window) -> bool) -> Duration {
    let find = |window: &Window| window.query_all_by_role_and_label(role, label).next().map(|node| node.rect());
    let started = Instant::now();
    loop {
        window.run();
        if done(window) {
            return started.elapsed();
        }
        let journal = || window.state().core.view().journal;
        assert!(started.elapsed() < LIMIT, "in time: what a click of {label} opens: {:#?}", journal().entries);
        if let Some(rect) = find(window) {
            press_in_place(window, rect.center(), find);
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
