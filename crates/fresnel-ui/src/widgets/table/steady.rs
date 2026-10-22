//! The rows of a table stay in their place under the pointer.
//!
//! egui gives a press to the widget that the frame before the press had at the pointer, and a
//! person aims at a row of a frame some time before the press. A note above the table goes away
//! when the store of a cluster becomes live, and the rows then move up by its height: a click
//! that a person aimed at a row opened the row below it. So while the pointer is over the
//! table, the table keeps the top of the frame before, and the place of the note stays empty
//! until the pointer leaves the table. A part above the table that comes moves the rows down
//! at once: a new note tells a change of a cluster.

use egui::{Rect, Ui};

/// The place of the table in the last frame that showed it.
#[derive(Default)]
pub(super) struct Steady {
    last: Option<Rect>,
}

impl Steady {
    /// Adds the space that keeps the top of the table where the last frame had it, while the
    /// pointer is over the table and the parts above it take less place than in that frame.
    pub(super) fn hold(&mut self, ui: &mut Ui) {
        let top = ui.available_rect_before_wrap().top();
        let pointer = ui.input(|input| input.pointer.hover_pos());
        let held = self.last.filter(|last| pointer.is_some_and(|place| last.contains(place)));
        if let Some(gap) = held.map(|last| last.top() - top).filter(|gap| *gap > 0.0) {
            ui.add_space(gap);
        }
        self.last = Some(ui.available_rect_before_wrap());
    }

    /// Forgets the place: the view does not show the table in this frame.
    pub(super) fn rest(&mut self) {
        self.last = None;
    }
}
