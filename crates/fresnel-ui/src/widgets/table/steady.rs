//! The rows of a table stay in their place under the pointer.
//!
//! egui gives a press to the widget that the frame before the press had at the pointer, and a
//! person aims at a row of a frame some time before the press. A note above the table comes or
//! goes away when the store of a cluster changes its state, and the rows then moved by its
//! height: a click that a person aimed at a row opened the row above or below it. So while
//! the pointer is over the part of the table that shows, or a press that began there lasts,
//! the table keeps the top of the frame before. The place of a note that went away stays
//! empty. A note that comes shows at once over the top of the table: the table paints nothing
//! and takes no press above the rest of the place (`Place::shown`). A pointer over that note
//! is not over the table, so the table then takes the rest of the place again, and the note
//! gets its presses, as the drag of its scroll bar. A key that moves the cursor ends the hold
//! too: the table scrolls to the row of the cursor, which a note could cover.
//!
//! `egui_table` reads the scroll offset of its rows from the clip of each of its parts, so a
//! clip that cuts the top of the rows moves them. The clip of the table starts at its top, and
//! each title, row and cell cuts itself to `shown` (`delegate.rs`). The scroll area of
//! `egui_table` paints its scroll bar and its fades from the top of the table too, so `cover`
//! paints the parts above the table again over the place that they cover.

use egui::epaint::ClippedShape;
use egui::layers::{PaintList, ShapeIdx};
use egui::{Rect, Shape, Ui, pos2};

/// The place of the table in the last frame that showed it.
#[derive(Default)]
pub(super) struct Steady {
    last: Option<Place>,
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

impl Place {
    /// The part of the table that shows: a note that came covers the rest.
    fn visible(&self) -> Rect {
        self.table.intersect(self.shown)
    }

    /// The part of the table that a note that came covers, if one does.
    fn covered(&self) -> Option<Rect> {
        let bottom = self.shown.top().min(self.table.bottom());
        (bottom > self.table.top()).then(|| Rect::from_x_y_ranges(self.table.x_range(), self.table.top()..=bottom))
    }
}

impl Steady {
    /// The place of the table in this frame: the rest of `ui`, with the top of the last frame
    /// while the pointer is over the part of that frame that showed, or a press that began
    /// there lasts.
    pub(super) fn place(&mut self, ui: &Ui) -> Place {
        let (rest, parent) = (ui.available_rect_before_wrap(), ui.clip_rect());
        let (pointer, press) = ui.input(|input| (input.pointer.hover_pos(), input.pointer.press_origin()));
        let held = self.last.filter(|last| pointer.into_iter().chain(press).any(|at| last.visible().contains(at)));
        let top = held.map_or(rest.top(), |last| last.table.top().min(rest.bottom()));
        let table = Rect::from_min_max(pos2(rest.left(), top), rest.max);
        let clip = Rect::from_min_max(pos2(parent.left(), parent.top().min(top)), parent.max);
        let place = Place { table, shown: rest.intersect(parent), clip };
        self.last = Some(place);
        place
    }

    /// Forgets the place: the view does not show the table in this frame, or a key moved the
    /// cursor.
    pub(super) fn rest(&mut self) {
        self.last = None;
    }
}

/// The index of the next shape of the layer of `ui`: the first shape of a table drawn now.
pub(super) fn next_shape(ui: &Ui) -> ShapeIdx {
    ui.ctx().graphics(|layers| layers.get(ui.layer_id()).map_or(ShapeIdx(0), PaintList::next_idx))
}

/// Paints over the part of `place` that a note covers what the frame painted there before the
/// table, whose first shape has the index `start`: the background of the page and then each
/// shape in sequence, cut to that part. The parts above the table then show there as if the
/// table were not under them.
pub(super) fn cover(ui: &Ui, start: ShapeIdx, place: &Place) {
    let Some(covered) = place.covered().filter(|_| ui.is_visible()) else {
        return;
    };
    let background = Shape::rect_filled(covered, 0.0, ui.stack().bg_color());
    ui.ctx().graphics_mut(|layers| {
        let list = layers.entry(ui.layer_id());
        let earlier: Vec<ClippedShape> = list
            .all_entries()
            .take(start.0)
            .filter_map(|clipped| {
                let clip_rect = clipped.clip_rect.intersect(covered);
                let shown = clip_rect.intersect(clipped.shape.visual_bounding_rect()).is_positive();
                shown.then(|| ClippedShape { clip_rect, shape: clipped.shape.clone() })
            })
            .collect();
        list.add(covered, background);
        for ClippedShape { clip_rect, shape } in earlier {
            list.add(clip_rect, shape);
        }
    });
}
