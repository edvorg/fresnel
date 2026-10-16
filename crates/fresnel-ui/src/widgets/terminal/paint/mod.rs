//! Paints the copy of a screen: the background runs, one galley for each run of one style,
//! the box characters as shapes, and the cursor.
//!
//! Each run starts at the exact x of its first column, and a character that can come from a
//! fallback font stands alone in its cells: no glyph moves the cells after it. One galley for
//! each run of a style took 1.5 ms for a screen of 200 x 60 in the spike.

mod boxes;
mod cursor;
mod row;

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Galley, Painter, Pos2, Rect, Shape, Stroke, Vec2};
use fresnel_core::config::Rgb;
use fresnel_core::terminal::Frame;

use super::metrics::CellMetrics;
use row::RowPainter;

/// How a frame looks: the fonts and the colors of the app.
pub(super) struct Look {
    pub(super) regular: FontId,
    pub(super) bold: FontId,
    /// False with no bold font: a bold run is painted two times, one pixel apart.
    pub(super) bold_face: bool,
    pub(super) background: Color32,
    pub(super) selection: Color32,
    pub(super) cursor: Color32,
    /// With no focus, the cursor is a hollow block.
    pub(super) focused: bool,
}

/// Paints `frame` with its top left corner at the top left corner of `area`. The part of
/// `area` that is less than a cell gets the background.
pub(super) fn paint(painter: &Painter, frame: &Frame, (area, metrics): (Rect, CellMetrics), look: &Look) {
    let pen = Pen { painter, origin: area.min, metrics, look };
    let mut layers = Layers::default();
    for row in 0..usize::from(frame.size.rows) {
        let mut row_painter = RowPainter::new(&pen, &mut layers, row);
        for (column, cell) in frame.row(row).iter().enumerate() {
            row_painter.cell(column, cell, frame.marks(row, column));
        }
        row_painter.finish(usize::from(frame.size.columns));
    }
    if let Some(at) = frame.cursor {
        cursor::paint(&pen, &mut layers, frame, at);
    }
    painter.rect_filled(area, 0.0, look.background);
    painter.extend(layers.backgrounds);
    painter.extend(layers.shapes);
    painter.extend(layers.text);
    painter.extend(layers.cursor);
}

/// The shapes in the sequence of the paint. Each background comes before each text, so a
/// glyph that is taller than its cell is not cut by the background of the next row.
#[derive(Default)]
struct Layers {
    backgrounds: Vec<Shape>,
    /// The box characters.
    shapes: Vec<Shape>,
    text: Vec<Shape>,
    cursor: Vec<Shape>,
}

/// The style of a run of text.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TextStyle {
    color: Color32,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

impl TextStyle {
    /// A blank cell with a line through it or under it shows.
    fn marks_blanks(&self) -> bool {
        self.underline || self.strike
    }
}

/// What all rows of one frame share.
struct Pen<'a> {
    painter: &'a Painter,
    origin: Pos2,
    metrics: CellMetrics,
    look: &'a Look,
}

impl Pen<'_> {
    /// A galley of `text`. `spaced`: the glyphs keep to the columns of the grid.
    fn galley(&self, text: String, style: TextStyle, spaced: bool) -> Arc<Galley> {
        let bold = style.bold && self.look.bold_face;
        let spacing = match (spaced, bold) {
            (false, _) => 0.0,
            (true, false) => self.metrics.spacing,
            (true, true) => self.metrics.bold_spacing,
        };
        let line = |on: bool| if on { Stroke::new(self.metrics.height / 16.0, style.color) } else { Stroke::NONE };
        let format = TextFormat {
            font_id: if bold { self.look.bold.clone() } else { self.look.regular.clone() },
            color: style.color,
            italics: style.italic,
            underline: line(style.underline),
            strikethrough: line(style.strike),
            extra_letter_spacing: spacing,
            line_height: Some(self.metrics.height),
            ..TextFormat::default()
        };
        self.painter.layout_job(LayoutJob::single_section(text, format))
    }

    fn push_text(&self, layers: &mut Layers, pos: Pos2, galley: Arc<Galley>, style: TextStyle) {
        if style.bold && !self.look.bold_face {
            // A second copy one pixel to the right makes the stems thicker.
            let offset = Vec2::new(1.0 / self.painter.pixels_per_point(), 0.0);
            layers.text.push(Shape::galley(pos + offset, Arc::clone(&galley), style.color));
        }
        layers.text.push(Shape::galley(pos, galley, style.color));
    }
}

pub(super) fn color(Rgb([r, g, b]): Rgb) -> Color32 {
    Color32::from_rgb(r, g, b)
}
