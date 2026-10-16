//! One row of the grid to shapes: the runs of a background, the runs of a style, the
//! characters that stand alone and the box characters.

use egui::{Color32, Pos2, Shape};
use fresnel_core::terminal::{Cell, CellFlags};

use super::boxes::{self, Pen as BoxPen};
use super::{Layers, Pen, TextStyle, color};

struct TextRun {
    start: usize,
    style: TextStyle,
    text: String,
}

pub(super) struct RowPainter<'a> {
    pen: &'a Pen<'a>,
    layers: &'a mut Layers,
    row: usize,
    /// The open run of text.
    text: Option<TextRun>,
    /// The first column and the color of the open run of a background.
    background: Option<(usize, Color32)>,
}

impl<'a> RowPainter<'a> {
    pub(super) fn new(pen: &'a Pen<'a>, layers: &'a mut Layers, row: usize) -> Self {
        Self { pen, layers, row, text: None, background: None }
    }

    /// Adds the next cell of the row, from left to right.
    pub(super) fn cell(&mut self, column: usize, cell: &Cell, marks: Option<&str>) {
        let look = self.pen.look;
        let background = if cell.flags.contains(CellFlags::SELECTED) { look.selection } else { color(cell.bg) };
        self.background_cell(column, background);
        if cell.flags.contains(CellFlags::SPACER) {
            return;
        }
        let style = TextStyle {
            color: color(cell.fg),
            bold: cell.flags.contains(CellFlags::BOLD),
            italic: cell.flags.contains(CellFlags::ITALIC),
            underline: cell.flags.contains(CellFlags::UNDERLINE),
            strike: cell.flags.contains(CellFlags::STRIKE),
        };
        let columns = if cell.flags.contains(CellFlags::WIDE) { 2 } else { 1 };
        match (cell.c, marks) {
            (' ', None) => self.blank_cell(column, style),
            (c, None) if c.is_ascii_graphic() => self.run_cell(column, c, style),
            (c, None) if boxes::is_shape(c) => self.shape(column, c, style),
            (c, marks) => {
                self.flush_text();
                let mut text = String::from(c);
                text.push_str(marks.unwrap_or_default());
                self.alone(column, columns, text, style);
            }
        }
    }

    /// Closes the open runs at the end of the row.
    pub(super) fn finish(&mut self, columns: usize) {
        self.flush_text();
        self.flush_background(columns);
    }

    fn background_cell(&mut self, column: usize, color: Color32) {
        if self.background.is_some_and(|(_, open)| open == color) {
            return;
        }
        self.flush_background(column);
        if color != self.pen.look.background {
            self.background = Some((column, color));
        }
    }

    fn flush_background(&mut self, end: usize) {
        if let Some((start, color)) = self.background.take() {
            let rect = self.pen.metrics.cells_rect(self.pen.origin, start, self.row, end - start);
            self.layers.backgrounds.push(Shape::rect_filled(rect, 0.0, color));
        }
    }

    fn run_cell(&mut self, column: usize, c: char, style: TextStyle) {
        match &mut self.text {
            Some(run) if run.style == style => run.text.push(c),
            _ => {
                self.flush_text();
                self.text = Some(TextRun { start: column, style, text: String::from(c) });
            }
        }
    }

    /// A blank stays in the open run of the same style, so a line of plain words is one
    /// galley; else it ends the run.
    fn blank_cell(&mut self, column: usize, style: TextStyle) {
        match &mut self.text {
            Some(run) if run.style == style => run.text.push(' '),
            _ => {
                self.flush_text();
                if style.marks_blanks() {
                    self.text = Some(TextRun { start: column, style, text: String::from(' ') });
                }
            }
        }
    }

    fn flush_text(&mut self) {
        let Some(mut run) = self.text.take() else { return };
        if !run.style.marks_blanks() {
            run.text.truncate(run.text.trim_end_matches(' ').len());
        }
        let galley = self.pen.galley(run.text, run.style, true);
        let pos = self.pen.metrics.cell_pos(self.pen.origin, run.start, self.row);
        self.pen.push_text(self.layers, pos, galley, run.style);
    }

    /// A box character, a block or a braille pattern as shapes. It ends the run of text: the
    /// next character is right of its cell.
    fn shape(&mut self, column: usize, c: char, style: TextStyle) {
        self.flush_text();
        let rect = self.pen.metrics.cells_rect(self.pen.origin, column, self.row, 1);
        let pixel = 1.0 / self.pen.painter.pixels_per_point();
        let mut pen = BoxPen { rect, color: style.color, pixel, shapes: &mut self.layers.shapes };
        boxes::draw(c, &mut pen);
    }

    /// One glyph in the middle of its one or two cells.
    fn alone(&mut self, column: usize, columns: usize, text: String, style: TextStyle) {
        let galley = self.pen.galley(text, style, false);
        let cells = self.pen.metrics.cells_rect(self.pen.origin, column, self.row, columns);
        let pos = Pos2::new(cells.center().x - galley.size().x / 2.0, cells.min.y);
        self.pen.push_text(self.layers, pos, galley, style);
    }
}
