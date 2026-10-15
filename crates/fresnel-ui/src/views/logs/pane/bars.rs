//! The scroll bars of the pane. The pane has no scroll area of egui, which needs the height
//! of each row: a bar measures lines, not points, so the pane lays out no line off the screen.

use egui::{Id, Rect, Sense, Ui, pos2};

use crate::theme;

/// The width of a bar.
pub(crate) const WIDTH: f32 = 10.0;
/// The shortest handle, so that a view of 200,000 lines has a handle to drag.
const SMALLEST: f32 = 24.0;

/// One bar: the part `start .. start + visible` of `total` is on the screen.
pub(crate) struct Bar {
    pub(crate) rect: Rect,
    pub(crate) vertical: bool,
    pub(crate) start: f32,
    pub(crate) visible: f32,
    pub(crate) total: f32,
}

impl Bar {
    /// Draws the bar. Gives the new start after a drag of the handle or a click on the track.
    pub(crate) fn show(&self, ui: &mut Ui, id: Id) -> Option<f32> {
        if self.total <= self.visible || self.total <= 0.0 {
            return None;
        }
        let palette = theme::palette(ui);
        let response = ui.interact(self.rect, id, Sense::click_and_drag());
        let along = |rect: Rect| if self.vertical { (rect.top(), rect.height()) } else { (rect.left(), rect.width()) };
        let (track_start, track) = along(self.rect);
        let handle = (self.visible / self.total * track).clamp(SMALLEST.min(track), track);
        let room = (track - handle).max(1.0);
        let part = (self.start / (self.total - self.visible)).clamp(0.0, 1.0);
        let handle_start = track_start + part * room;
        let handle_rect = if self.vertical {
            Rect::from_x_y_ranges(self.rect.x_range(), handle_start..=handle_start + handle)
        } else {
            Rect::from_x_y_ranges(handle_start..=handle_start + handle, self.rect.y_range())
        };
        let painter = ui.painter_at(self.rect);
        painter.rect_filled(self.rect, 0.0, palette.bg_chrome);
        let color = if response.dragged() || response.hovered() { palette.text_faint } else { palette.border };
        painter.rect_filled(handle_rect.shrink(2.0), 3.0, color);
        let pointer = response.interact_pointer_pos()?;
        let at = if self.vertical { pointer.y } else { pointer.x };
        let part = ((at - track_start - handle / 2.0) / room).clamp(0.0, 1.0);
        (response.dragged() || response.clicked()).then_some(part * (self.total - self.visible))
    }
}

/// The rect of a vertical bar at the right of `rect`, and of a horizontal one at its bottom.
pub(crate) fn places(rect: Rect, horizontal: bool) -> (Rect, Option<Rect>) {
    let bottom = if horizontal { rect.bottom() - WIDTH } else { rect.bottom() };
    let vertical = Rect::from_min_max(pos2(rect.right() - WIDTH, rect.top()), pos2(rect.right(), bottom));
    let horizontal =
        horizontal.then(|| Rect::from_min_max(pos2(rect.left(), bottom), pos2(rect.right() - WIDTH, rect.bottom())));
    (vertical, horizontal)
}
