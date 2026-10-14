//! The edge at the right of each column: a handle that changes the width of that column
//! with a drag, and fits it to its cells with a double click. The table owns the handles,
//! not `egui_table`:
//!
//! - A drag starts at the press. A handle that also takes clicks starts its drag only after
//!   a move of 6 points, and the edge stays behind the pointer until then.
//! - The width of a drag comes before the layout of a frame, so the edge is at the pointer
//!   in the frame of each move. `egui_table` changes the width after its layout.
//! - A width changes by the move of the pointer after the press. A click changes nothing,
//!   and Escape gives the column the width that it had at the press. That is the width of
//!   the table, not the one on the screen: a narrow window shows less of the first column,
//!   and a move to the right cannot make that column wider on the screen.
//! - The handle is on top of the titles and the rows, and it takes no click: a click on
//!   an edge does not sort and does not select a row.
//! - The handle ends at the scroll bars of the rows, so a drag of a scroll bar scrolls.

use egui::{CursorIcon, Id, Key, PointerButton, Rect, Sense, Stroke, Ui};

/// A drag of an edge, from its press to its release.
#[derive(Clone, Copy, Debug)]
struct Drag {
    column: usize,
    id: Id,
    /// The width of the column on the screen at the press.
    shown: f32,
    /// The width of the column in the table at the press.
    kept: f32,
    /// Where the press was.
    press: f32,
}

impl Drag {
    /// The width of the column with the pointer at `x`. A column that the window cuts is as
    /// wide on the screen as the table lets it be: a move to the right keeps its width.
    fn width(&self, x: f32) -> DragWidth {
        let cut = self.kept > self.shown;
        if x == self.press || (cut && x > self.press) {
            DragWidth::Kept(self.kept)
        } else {
            DragWidth::Moved(self.shown + x - self.press)
        }
    }
}

/// The width that a drag gives its column in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum DragWidth {
    /// The width on the screen at the press plus the move of the pointer.
    Moved(f32),
    /// The width of the table at the press: the pointer is at the press, Escape ended the
    /// drag, or the pointer is right of the press at a column that the window cuts.
    Kept(f32),
}

/// One edge on the screen: the right side of a column.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Edge {
    column: usize,
    x: f32,
}

/// What one frame of the table showed: the edge of each title that shows, the top of the
/// titles, and the right side and the bottom of the place of the rows. A scroll bar is no
/// part of that place.
#[derive(Debug, Default)]
pub(super) struct Seen {
    edges: Vec<Edge>,
    top: Option<f32>,
    rows: Option<(f32, f32)>,
}

impl Seen {
    /// Takes a title of the frame, and its edge if it shows. `width` is the width of its
    /// column in this frame: a pass that measures lays a title out narrower.
    pub(super) fn title(&mut self, ui: &Ui, column: usize, width: Option<f32>) {
        self.top = Some(ui.max_rect().top());
        if let Some(edge) = width.and_then(|width| of_title(ui, column, width)) {
            self.edges.push(edge);
        }
    }

    /// Takes a row of the frame. The rows of the part that scrolls end at its scroll bars,
    /// the rows of the first column at the bottom of the table.
    pub(super) fn row(&mut self, ui: &Ui) {
        let clip = ui.clip_rect();
        let (right, bottom) = self.rows.unwrap_or((clip.right(), clip.bottom()));
        self.rows = Some((right.max(clip.right()), bottom.min(clip.bottom())));
    }
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
    pub(super) fn dragged(&mut self, ui: &Ui) -> Option<(usize, DragWidth)> {
        let drag = self.drag?;
        let ctx = ui.ctx();
        if ctx.is_being_dragged(drag.id) {
            let pointer = ui.input(|input| input.pointer.latest_pos())?;
            return Some((drag.column, drag.width(pointer.x)));
        }
        self.drag = None;
        if ctx.drag_stopped_id() != Some(drag.id) {
            return None;
        }
        // The release can come with a last move.
        let (escape, pointer) = ui.input(|input| (input.key_pressed(Key::Escape), input.pointer.latest_pos()));
        let width = if escape { DragWidth::Kept(drag.kept) } else { drag.width(pointer?.x) };
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

    /// Puts a handle on each edge that the frame showed, after the layout of the table, and
    /// draws its line. `table` is the place of the table: with no row on the screen, the
    /// lines end at its bottom. `id` names the handle of a column, and `widths` gives the
    /// width of a column on the screen and in the table.
    pub(super) fn handles(
        &mut self,
        ui: &Ui,
        seen: &Seen,
        table: Rect,
        id: impl Fn(usize) -> Id,
        widths: impl Fn(usize) -> Option<(f32, f32)>,
    ) {
        let Some(top) = seen.top else {
            return;
        };
        let (right, bottom) = seen.rows.unwrap_or((table.right(), table.bottom()));
        let grab = ui.style().interaction.resize_grab_radius_side;
        let busy = ui.input(|input| input.pointer.any_down());
        for edge in seen.edges.iter().filter(|edge| edge.x <= right) {
            let rect = Rect::from_x_y_ranges(edge.x - grab..=(edge.x + grab).min(right), top..=bottom);
            let response = ui.interact(rect, id(edge.column), Sense::drag());
            if response.drag_started()
                && let (Some(press), Some((shown, kept))) =
                    (ui.input(|input| input.pointer.press_origin()), widths(edge.column))
            {
                self.drag = Some(Drag { column: edge.column, id: response.id, shown, kept, press: press.x });
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
/// side has its edge there only right of the fixed first column.
fn of_title(ui: &Ui, column: usize, width: f32) -> Option<Edge> {
    let x = ui.max_rect().left() + width;
    let clip = ui.clip_rect();
    (x > clip.left() && x <= clip.right()).then_some(Edge { column, x })
}
