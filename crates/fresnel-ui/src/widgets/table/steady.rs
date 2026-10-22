//! The rows of a table stay in their place under the pointer.
//!
//! egui gives a press to the widget that the frame before the press had at the pointer, and a
//! person aims at a row of a frame some time before the press. A note above the table comes or
//! goes away when the store of a cluster changes its state, and the rows then moved by its
//! height: a click that a person aimed at a row opened the row above or below it. So while
//! the pointer is over the table or a press on it lasts, the table keeps the top of the frame
//! before. The place of a note that went away stays empty. A note that comes shows at once
//! over the top of the table: the table paints nothing and takes no press above the rest of
//! the place (`Place::shown`). After that, the table takes the rest of the place again.
//!
//! `egui_table` reads the scroll offset of its rows from the clip of each of its parts, so a
//! clip that cuts the top of the rows moves them. The clip of the table starts at its top, and
//! each title, row and cell cuts itself to `shown` (`delegate.rs`).

use egui::{Rect, Ui, pos2};

/// The place of the table in the last frame that showed it.
#[derive(Default)]
pub(super) struct Steady {
    last: Option<Rect>,
}

/// Where a table is in one frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct Place {
    /// The place of the titles and the rows.
    pub(super) table: Rect,
    /// The rest of the place below the parts above the table: the table shows only there.
    pub(super) shown: Rect,
    /// The clip of `egui_table`: the clip of the parent from the top of the table.
    pub(super) clip: Rect,
}

impl Steady {
    /// The place of the table in this frame: the rest of `ui`, with the top of the last frame
    /// while the pointer is over the place of that frame, or a press that began there lasts.
    pub(super) fn place(&mut self, ui: &Ui) -> Place {
        let (rest, parent) = (ui.available_rect_before_wrap(), ui.clip_rect());
        let (pointer, press) = ui.input(|input| (input.pointer.hover_pos(), input.pointer.press_origin()));
        let held = self.last.filter(|last| pointer.into_iter().chain(press).any(|at| last.contains(at)));
        let top = held.map_or(rest.top(), |last| last.top().min(rest.bottom()));
        let table = Rect::from_min_max(pos2(rest.left(), top), rest.max);
        self.last = Some(table);
        let clip = Rect::from_min_max(pos2(parent.left(), parent.top().min(top)), parent.max);
        Place { table, shown: rest.intersect(parent), clip }
    }

    /// Forgets the place: the view does not show the table in this frame.
    pub(super) fn rest(&mut self) {
        self.last = None;
    }
}
