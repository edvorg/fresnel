//! The size of one cell, from the font.

use egui::{FontId, Pos2, Rect, Ui, Vec2};
use fresnel_core::terminal::{GridSize, Spot};

/// The size of a cell in points. Its width and height are whole pixels, so each column and
/// each row starts on the border of a pixel, each glyph looks the same, and the lines of box
/// characters join.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CellMetrics {
    pub(super) width: f32,
    pub(super) height: f32,
    /// The space that the layout of a run adds after each glyph of the regular face, from its
    /// advance to the cell width.
    pub(super) spacing: f32,
    /// The same for the bold face, whose advance can differ.
    pub(super) bold_spacing: f32,
    pub(super) pixels_per_point: f32,
}

impl CellMetrics {
    pub(super) fn measure(ui: &Ui, regular: &FontId, bold: &FontId) -> Self {
        let pixels_per_point = ui.pixels_per_point();
        let (advance, bold_advance, row_height) = ui.fonts_mut(|fonts| {
            (fonts.glyph_width(regular, 'M'), fonts.glyph_width(bold, 'M'), fonts.row_height(regular))
        });
        let whole = |points: f32| ((points * pixels_per_point).ceil() / pixels_per_point).max(1.0);
        let width = whole(advance);
        Self {
            width,
            height: whole(row_height),
            spacing: width - advance,
            bold_spacing: width - bold_advance,
            pixels_per_point,
        }
    }

    /// The count of whole cells in `area`.
    pub(super) fn grid_size(&self, area: Vec2) -> GridSize {
        let count = |length: f32, cell: f32| (length / cell).floor().clamp(0.0, f32::from(u16::MAX)) as u16;
        GridSize::new(count(area.x, self.width), count(area.y, self.height))
    }

    /// The size of a cell in pixels, for a program that asks for it.
    pub(super) fn pixels(&self) -> (u16, u16) {
        let pixels = |points: f32| (points * self.pixels_per_point).round().clamp(0.0, f32::from(u16::MAX)) as u16;
        (pixels(self.width), pixels(self.height))
    }

    /// The top left corner of a cell.
    pub(super) fn cell_pos(&self, origin: Pos2, column: usize, row: usize) -> Pos2 {
        Pos2::new(origin.x + column as f32 * self.width, origin.y + row as f32 * self.height)
    }

    /// The rectangle of `columns` cells of one row.
    pub(super) fn cells_rect(&self, origin: Pos2, column: usize, row: usize, columns: usize) -> Rect {
        Rect::from_min_size(self.cell_pos(origin, column, row), Vec2::new(columns as f32 * self.width, self.height))
    }

    /// The cell under a place, and its half. A place out of the grid gives its nearest cell.
    pub(super) fn spot(&self, origin: Pos2, pos: Pos2) -> Spot {
        let x = ((pos.x - origin.x) / self.width).max(0.0);
        let y = ((pos.y - origin.y) / self.height).max(0.0);
        Spot { row: y as usize, column: x as usize, right: x.fract() >= 0.5 }
    }
}
