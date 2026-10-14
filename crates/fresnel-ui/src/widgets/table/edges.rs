//! The edge at the right of each column: a handle that changes the width of that column
//! with a drag, and fits it to its cells with a double click. The table owns the handles,
//! not `egui_table`:
//!
//! - A drag starts at the press. A handle that also takes clicks starts its drag only after
//!   a move of 6 points, and the edge stays behind the pointer until then.
//! - The width of a drag comes before the layout of a frame, so the edge is at the pointer
//!   in the frame of each move. `egui_table` changes the width after its layout.
//! - A width changes by the move of the pointer after the press. A click changes nothing,
//!   and Escape gives the column the width that it had at the press.
//! - The handle is on top of the titles and the rows, and it takes no click: a click on
//!   an edge does not sort and does not select a row.

use egui::{CursorIcon, Id, Key, PointerButton, Rect, Sense, Stroke, Ui};

/// A drag of an edge, from its press to its release.
#[derive(Clone, Copy, Debug)]
struct Drag {
    column: usize,
    id: Id,
    /// The width of the column on the screen at the press.
    width: f32,
    /// Where the press was.
    press: f32,
}

/// One edge on the screen: the right side of a column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Edge {
    pub(super) column: usize,
    pub(super) x: f32,
}

/// The handles of one table.
#[derive(Debug, Default)]
pub(super) struct Edges {
    drag: Option<Drag>,
    /// The column that the next frame fits to its cells.
    fit: Option<usize>,
}

impl Edges {
    /// The width of the column of the drag in this frame, from the pointer now. Call it
    /// before the layout of the frame.
    pub(super) fn dragged(&mut self, ui: &Ui) -> Option<(usize, f32)> {
        let drag = self.drag?;
        let ctx = ui.ctx();
        if ctx.is_being_dragged(drag.id) {
            let pointer = ui.input(|input| input.pointer.latest_pos())?;
            return Some((drag.column, drag.width + pointer.x - drag.press));
        }
        self.drag = None;
        if ctx.drag_stopped_id() != Some(drag.id) {
            return None;
        }
        // The release can come with a last move.
        let (escape, pointer) = ui.input(|input| (input.key_pressed(Key::Escape), input.pointer.latest_pos()));
        let width = if escape { drag.width } else { drag.width + pointer?.x - drag.press };
        Some((drag.column, width))
    }

    /// The column to fit to its cells in this frame, after a double click on its edge.
    pub(super) fn take_fit(&mut self) -> Option<usize> {
        self.fit.take()
    }

    /// Forgets the drag: the table gave up its rows.
    pub(super) fn rest(&mut self) {
        self.drag = None;
        self.fit = None;
    }

    /// Puts a handle on each edge of this frame, after the layout of the table, and draws
    /// its line from `top` to `bottom`. `id` names the handle of a column, and `widths`
    /// gives the width of each column in this frame.
    pub(super) fn handles(
        &mut self,
        ui: &Ui,
        edges: &[Edge],
        span: (f32, f32),
        id: impl Fn(usize) -> Id,
        widths: &[f32],
    ) {
        let (top, bottom) = span;
        let grab = ui.style().interaction.resize_grab_radius_side;
        let busy = ui.input(|input| input.pointer.any_down());
        for edge in edges {
            let rect = Rect::from_x_y_ranges(edge.x - grab..=edge.x + grab, top..=bottom);
            let response = ui.interact(rect, id(edge.column), Sense::drag());
            if response.drag_started()
                && let (Some(press), Some(width)) =
                    (ui.input(|input| input.pointer.press_origin()), widths.get(edge.column))
            {
                self.drag = Some(Drag { column: edge.column, id: response.id, width: *width, press: press.x });
            }
            let double_click = ui.input(|input| input.pointer.button_double_clicked(PointerButton::Primary));
            if double_click && response.contains_pointer() {
                self.fit = Some(edge.column);
                ui.ctx().request_repaint();
            }
            let hovered = response.hovered() && !busy;
            if hovered || response.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeColumn);
            }
            ui.painter().vline(edge.x, top..=bottom, stroke(ui, response.dragged(), hovered));
        }
    }
}

/// The line of an edge: brighter under the pointer and during its drag.
fn stroke(ui: &Ui, dragged: bool, hovered: bool) -> Stroke {
    let widgets = &ui.visuals().widgets;
    if dragged {
        widgets.active.bg_stroke
    } else if hovered {
        widgets.hovered.bg_stroke
    } else {
        widgets.noninteractive.bg_stroke
    }
}

/// The edge at the right of a title, if it shows: a title in the part that scrolls to the
/// side has its edge there only right of the fixed first column. `width` is the width of
/// the column in this frame: a pass that measures lays a title out narrower.
pub(super) fn of_title(ui: &Ui, column: usize, width: f32) -> Option<Edge> {
    let x = ui.max_rect().left() + width;
    let clip = ui.clip_rect().x_range();
    (x > clip.min && x <= clip.max).then_some(Edge { column, x })
}
