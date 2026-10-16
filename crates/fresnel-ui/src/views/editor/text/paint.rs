//! The rows of the editor in one frame: the galley of each line on the screen, the gutter
//! with the line numbers and the marks, the selection, the hits, the cursor, and a node of
//! the accessibility tree for each row.
//!
//! A line of more than 2,000 bytes shows its part around the screen, at the place of its
//! columns: a galley of a line of megabytes would cost seconds, and its triangles each frame.

use std::sync::Arc;

use egui::epaint::text::cursor::CCursor;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, FontId, Galley, Id, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use fresnel_core::manifest::check::{Level, Mark};

use super::buffer::{Buffer, Pos};
use super::lexer::{self, State, Token};
use super::moves::{self, Selection};
use super::{Overlay, colors};
use crate::theme::{self, Palette};

/// A line with more bytes shows its part around the screen.
const LONG: usize = 2_000;
/// The columns of a long line that show left and right of the screen.
const AROUND: usize = 64;
/// The space between the gutter and the text, and the room of a mark in the gutter.
const MARGIN: f32 = 8.0;
const MARK_ROOM: f32 = 12.0;

/// The sizes of the rows in one frame. Places are relative to the top left of the text.
pub(super) struct Metrics {
    font: FontId,
    pub(super) row: f32,
    pub(super) column: f32,
    gutter: f32,
    /// The left edge of the text, after the gutter.
    pub(super) text_left: f32,
    lines: usize,
}

impl Metrics {
    pub(super) fn of(ui: &Ui, lines: usize) -> Self {
        let font = theme::mono();
        let (row, column) = ui.fonts_mut(|fonts| (fonts.row_height(&font) + 2.0, fonts.glyph_width(&font, 'x')));
        let digits = lines.to_string().len().max(3);
        let gutter = MARK_ROOM + digits as f32 * column + MARGIN;
        Self { font, row, column: column.max(1.0), gutter, text_left: gutter + MARGIN, lines }
    }

    pub(super) fn height(&self) -> f32 {
        self.lines as f32 * self.row + self.row
    }
}

/// One line on the screen: the part of it that shows, and its tokens.
pub(super) struct Row {
    line: usize,
    /// The first column and the bytes of the part.
    start_column: usize,
    bytes: std::ops::Range<usize>,
    galley: Arc<Galley>,
    tokens: Vec<Token>,
}

/// The rows of the lines on the screen. `view` is the place of the content on the screen and
/// the part of it that shows.
pub(super) fn rows(ui: &Ui, metrics: &Metrics, view: (Pos2, Rect), buffer: &Buffer, states: &[State]) -> Vec<Row> {
    let (_, viewport) = view;
    let first = (viewport.top() / metrics.row).floor().max(0.0) as usize;
    let last = ((viewport.bottom() / metrics.row).ceil() as usize).min(buffer.count());
    (first..last).map(|line| row(ui, metrics, viewport, buffer, (states, line))).collect()
}

/// The row of one line.
fn row(ui: &Ui, metrics: &Metrics, viewport: Rect, buffer: &Buffer, (states, line): (&[State], usize)) -> Row {
    let text = buffer.line(line);
    let tokens = lexer::tokens(text, states.get(line).copied().unwrap_or_default());
    let (start_column, bytes) = if text.len() <= LONG {
        (0, 0..text.len())
    } else {
        let left = ((viewport.left() - metrics.text_left) / metrics.column).max(0.0) as usize;
        let start = left.saturating_sub(AROUND);
        let count = (viewport.width() / metrics.column) as usize + 2 * AROUND;
        (start, moves::byte_of_column(text, start)..moves::byte_of_column(text, start + count))
    };
    let palette = theme::palette(ui);
    let dark = ui.visuals().dark_mode;
    let mut job = LayoutJob::default();
    let mut at = bytes.start;
    let format = |color| TextFormat { font_id: metrics.font.clone(), color, ..TextFormat::default() };
    for token in &tokens {
        let (start, end) =
            (token.bytes.start.clamp(bytes.start, bytes.end), token.bytes.end.clamp(bytes.start, bytes.end));
        if start > at {
            job.append(text.get(at..start).unwrap_or_default(), 0.0, format(palette.text));
        }
        if end > start {
            job.append(text.get(start..end).unwrap_or_default(), 0.0, format(colors::of(token.kind, palette, dark)));
        }
        at = at.max(end);
    }
    if bytes.end > at {
        job.append(text.get(at..bytes.end).unwrap_or_default(), 0.0, format(palette.text));
    }
    Row { line, start_column, bytes, galley: ui.painter().layout_job(job), tokens }
}

/// The left edge of a place, from the left edge of the text.
pub(super) fn x_of(buffer: &Buffer, rows: &[Row], metrics: &Metrics, pos: Pos) -> f32 {
    let text = buffer.line(pos.line);
    match rows.iter().find(|row| row.line == pos.line) {
        Some(row) if row.bytes.contains(&pos.byte) || row.bytes.end == pos.byte => {
            let shown = text.get(row.bytes.start..pos.byte).map_or(0, |part| part.chars().count());
            let inside = row.galley.pos_from_cursor(CCursor::new(shown)).min.x;
            row.start_column as f32 * metrics.column + inside
        }
        _ => moves::column(text, pos.byte) as f32 * metrics.column,
    }
}

/// The place in the text under the pointer.
pub(super) fn pos_at(
    ui: &Ui,
    metrics: &Metrics,
    (origin, viewport): (Pos2, Rect),
    rows: &[Row],
    buffer: &Buffer,
    (states, pointer): (&[State], Pos2),
) -> Pos {
    let line = (((pointer.y - origin.y) / metrics.row).floor().max(0.0) as usize).min(buffer.count().saturating_sub(1));
    let x = pointer.x - origin.x - metrics.text_left;
    let text = buffer.line(line);
    let built;
    let row = match rows.iter().find(|row| row.line == line) {
        Some(row) => row,
        None => {
            built = row(ui, metrics, viewport, buffer, (states, line));
            &built
        }
    };
    let inside = x - row.start_column as f32 * metrics.column;
    let byte = if row.bytes.start == 0 && row.bytes.end == text.len() || (0.0..=row.galley.size().x).contains(&inside) {
        let index = row.galley.cursor_from_pos(vec2(inside, metrics.row / 2.0)).index.0;
        row.bytes.start + moves::byte_of_column(text.get(row.bytes.clone()).unwrap_or_default(), index)
    } else {
        moves::byte_of_column(text, (x / metrics.column).round().max(0.0) as usize)
    };
    Pos::new(line, byte)
}

/// The cursor and the selection, and whether the area has the focus.
pub(super) struct Cursor {
    pub(super) selection: Selection,
    pub(super) focused: bool,
}

/// Draws the rows: the line of the cursor, the hits, the selection, the text, the marks and
/// the cursor.
pub(super) fn draw(
    ui: &Ui,
    metrics: &Metrics,
    (origin, viewport): (Pos2, Rect),
    rows: &[Row],
    buffer: &Buffer,
    (cursor, overlay): (Cursor, Overlay<'_>),
) {
    let palette = theme::palette(ui);
    let screen = viewport.translate(origin.to_vec2());
    let painter = ui.painter_at(screen.with_min_x(screen.left() + metrics.gutter));
    let range = cursor.selection.range();
    for row in rows {
        let top = origin.y + row.line as f32 * metrics.row;
        let line_rect = Rect::from_min_max(pos2(screen.left(), top), pos2(screen.right(), top + metrics.row));
        let x = |byte: usize| origin.x + metrics.text_left + x_of(buffer, rows, metrics, Pos::new(row.line, byte));
        let span = |from: usize, to: usize, extra: f32| {
            Rect::from_min_max(pos2(x(from), top), pos2(x(to) + extra, top + metrics.row))
        };
        let marks = overlay.marks.iter().filter(|mark| mark.line == row.line + 1);
        if cursor.focused && row.line == cursor.selection.head.line {
            painter.rect_filled(line_rect, 0.0, palette.bg_raised.gamma_multiply(0.5));
        }
        if marks.clone().any(|mark| mark.level == Level::Error) {
            painter.rect_filled(line_rect, 0.0, palette.error.gamma_multiply(0.14));
        }
        let first = overlay.hits.partition_point(|hit| hit.line < row.line);
        for (index, hit) in overlay.hits.iter().enumerate().skip(first).take_while(|(_, hit)| hit.line == row.line) {
            let strong = overlay.current == Some(index);
            let color = palette.warn.gamma_multiply(if strong { 0.75 } else { 0.3 });
            painter.rect_filled(span(hit.bytes.start, hit.bytes.end, 0.0), 2.0, color);
        }
        if (range.start.line..=range.end.line).contains(&row.line) && range.start != range.end {
            let from = if row.line == range.start.line { range.start.byte } else { 0 };
            let (to, feed) = if row.line == range.end.line {
                (range.end.byte, 0.0)
            } else {
                (buffer.line(row.line).len(), metrics.column)
            };
            painter.rect_filled(span(from, to, feed), 0.0, ui.visuals().selection.bg_fill);
        }
        let text_at = pos2(origin.x + metrics.text_left + row.start_column as f32 * metrics.column, top + 1.0);
        painter.galley(text_at, Arc::clone(&row.galley), palette.text);
        for mark in marks {
            let start = moves::byte_of_column(buffer.line(row.line), mark.column.saturating_sub(1));
            let token = row.tokens.iter().find(|token| token.bytes.contains(&start) && token.bytes.end > start);
            let end = token.map_or(start + 1, |token| token.bytes.end).min(buffer.line(row.line).len()).max(start);
            let color = level_color(mark.level, palette);
            let under = top + metrics.row - 1.5;
            painter.hline(x(start)..=x(end).max(x(start) + metrics.column), under, Stroke::new(1.5, color));
        }
        if cursor.focused && row.line == cursor.selection.head.line {
            let head = x(cursor.selection.head.byte);
            painter.vline(head, top..=top + metrics.row, Stroke::new(2.0, palette.text));
        }
        node(ui, line_rect, Id::new(("editor row", row.line)), &row.galley.job.text);
    }
}

/// Draws the gutter at the left edge of the screen: the number of each row, and the mark of
/// a line with an error or a hint, whose text shows under the pointer.
pub(super) fn gutter(
    ui: &Ui,
    metrics: &Metrics,
    (origin, viewport): (Pos2, Rect),
    rows: &[Row],
    marks: &[Mark],
    current: usize,
) {
    let palette = theme::palette(ui);
    let screen = viewport.translate(origin.to_vec2());
    let rect = screen.with_max_x(screen.left() + metrics.gutter);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, palette.bg_surface);
    for row in rows {
        let top = origin.y + row.line as f32 * metrics.row;
        let color = if row.line == current { palette.text_muted } else { palette.text_faint };
        let at = pos2(rect.right() - MARGIN, top + metrics.row / 2.0);
        painter.text(at, Align2::RIGHT_CENTER, (row.line + 1).to_string(), metrics.font.clone(), color);
        if let Some(mark) = marks.iter().filter(|mark| mark.line == row.line + 1).min_by_key(|mark| mark.level) {
            painter.circle_filled(
                pos2(rect.left() + MARK_ROOM / 2.0, top + metrics.row / 2.0),
                3.5,
                level_color(mark.level, palette),
            );
        }
    }
    let response = ui.interact(rect, Id::new("editor gutter").with(ui.id()), Sense::hover());
    let pointed =
        response.hover_pos().map(|pointer| ((pointer.y - origin.y) / metrics.row).floor().max(0.0) as usize + 1);
    let texts: Vec<&str> =
        marks.iter().filter(|mark| Some(mark.line) == pointed).map(|mark| mark.text.as_str()).collect();
    if !texts.is_empty() {
        response.on_hover_text(texts.join("\n"));
    }
}

fn level_color(level: Level, palette: &Palette) -> egui::Color32 {
    match level {
        Level::Error => palette.error,
        Level::Hint => palette.warn,
    }
}

/// A node of the accessibility tree for a row: a screen reader reads the text that the
/// editor paints itself, and a test finds a line by its text. egui gives a place only to the
/// node of a widget that takes the focus, so the row sets its own.
fn node(ui: &Ui, rect: Rect, id: Id, text: &str) {
    let response = ui.interact(rect, ui.id().with(id), Sense::hover());
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(egui::accesskit::Role::ListItem);
        node.set_label(text);
        let (min, max) = (rect.min, rect.max);
        node.set_bounds(egui::accesskit::Rect {
            x0: min.x.into(),
            y0: min.y.into(),
            x1: max.x.into(),
            y1: max.y.into(),
        });
    });
}
