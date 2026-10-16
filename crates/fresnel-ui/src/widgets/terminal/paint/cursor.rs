//! The shapes of the cursor. Without the focus the cursor is a hollow block, so the user
//! sees that the keys go elsewhere.

use egui::{Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};
use fresnel_core::terminal::{CellFlags, Cursor, CursorShape, Frame};

use super::{Layers, Pen, TextStyle};

pub(super) fn paint(pen: &Pen<'_>, layers: &mut Layers, frame: &Frame, at: Cursor) {
    let columns = if at.wide { 2 } else { 1 };
    let metrics = pen.metrics;
    let rect = metrics.cells_rect(pen.origin, at.column, at.row, columns);
    let look = pen.look;
    let pixel = 1.0 / pen.painter.pixels_per_point();
    let shape = if look.focused { at.shape } else { CursorShape::HollowBlock };
    match shape {
        CursorShape::Block => {
            layers.cursor.push(Shape::rect_filled(rect, 0.0, look.cursor));
            let cell = frame.row(at.row).get(at.column);
            if let Some(cell) = cell.filter(|cell| cell.c != ' ' && !cell.flags.contains(CellFlags::SPACER)) {
                // The character under a block cursor shows in the background color.
                let style = TextStyle {
                    color: look.background,
                    bold: cell.flags.contains(CellFlags::BOLD),
                    italic: cell.flags.contains(CellFlags::ITALIC),
                    underline: false,
                    strike: false,
                };
                let mut text = String::from(cell.c);
                text.push_str(frame.marks(at.row, at.column).unwrap_or_default());
                let galley = pen.galley(text, style, false);
                let pos = Pos2::new(rect.center().x - galley.size().x / 2.0, rect.min.y);
                layers.cursor.push(Shape::galley(pos, galley, look.background));
            }
        }
        CursorShape::HollowBlock => {
            let stroke = Stroke::new(pixel.max(metrics.width / 8.0), look.cursor);
            layers.cursor.push(Shape::rect_stroke(rect, 0.0, stroke, StrokeKind::Inside));
        }
        CursorShape::Beam => {
            let beam = Rect::from_min_size(rect.min, Vec2::new((metrics.width / 6.0).max(pixel), rect.height()));
            layers.cursor.push(Shape::rect_filled(beam, 0.0, look.cursor));
        }
        CursorShape::Underline => {
            let height = (metrics.height / 8.0).max(pixel);
            let line = Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - height), rect.max);
            layers.cursor.push(Shape::rect_filled(line, 0.0, look.cursor));
        }
    }
}
