//! The pane of a log view: its own virtual rows (plan section 12.7, research 05 section 5.5).
//!
//! No label holds the lines: a view has up to 200,000 of them. The pane lays out the rows
//! on the screen only, one galley for each, so a frame costs the same with 20 or 200,000
//! lines. The scroll area of egui needs the height of each row and an `f32` for the sum;
//! the pane has its own position instead, a line and a row of it (`rows`), with bars that
//! measure lines (`bars`).
//!
//! Follow keeps the last line at the bottom. A scroll up ends it, and a scroll to the end
//! starts it again. The pane has its own selection (`select`), a row the colors of the
//! program, the hits of the search and the selection (`paint`, `colors`), and each row on
//! the screen a node of the accessibility tree with its text, for a screen reader.
//!
//! - `input`: the wheel, the keys, the bars and the pointer. `draw`: the rows of a frame.

mod bars;
mod colors;
mod draw;
mod input;
mod paint;
pub(crate) mod rows;
pub(crate) mod select;

use std::collections::VecDeque;
use std::ops::Range;

use egui::{FontId, Rect, Sense, Ui, pos2};
use fresnel_core::logs::LogView;

use self::rows::{Cut, Position, Shown};
use self::select::Selection;
use super::needle::Needle;
use crate::format::Clock;
use crate::theme;

/// The columns of the time: "08:00:00.123" and a space.
const TIME_COLUMNS: f32 = 13.0;
/// The most columns of the name of a source, for a view with more than one source: the full
/// name of a pod of a Deployment has some 26 characters. The gutter is as wide as its widest
/// name and a space, and for the pods of an object at least as wide as a rollout name
/// (`draw::source_columns`), so short names leave little gap before the text.
const SOURCE_COLUMNS: usize = 32;
/// The space between the text and the edges of the pane.
const MARGIN: f32 = 6.0;

/// What the pane shows in one frame.
pub(crate) struct Input<'a> {
    pub(crate) view: &'a LogView,
    /// The lines that the filter keeps. `None` with no filter.
    pub(crate) kept: Option<&'a VecDeque<u64>>,
    pub(crate) wrap: bool,
    pub(crate) times: bool,
    /// The needle of the search, for the marks of its hits.
    pub(crate) search: Option<&'a Needle>,
    /// The line of the current hit.
    pub(crate) current: Option<u64>,
    pub(crate) clock: &'a Clock,
}

impl Input<'_> {
    fn shown(&self) -> Shown<'_> {
        Shown { lines: &self.view.lines, kept: self.kept }
    }
}

/// What the user did in the pane in one frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Output {
    /// A click on the button of a long line: its number.
    pub(crate) full_line: Option<u64>,
    /// A copy key with a selection.
    pub(crate) copy: bool,
}

/// The state of the pane of one view.
#[derive(Debug)]
pub(crate) struct Pane {
    top: Position,
    /// The part of the top row above the pane, in points.
    offset: f32,
    /// True while the last line stays at the bottom.
    pub(crate) follow: bool,
    /// The start of the text with no wrap, in points from the left edge of the text.
    left: f32,
    pub(crate) selection: Option<Selection>,
    /// A line to bring into the pane in the next frame: the current hit of the search.
    reveal: Option<u64>,
    /// The rows of the last frame, for the place of the pointer.
    placed: Vec<Placed>,
    /// The width of the widest row of the last frame, for the bar with no wrap.
    widest: f32,
}

/// One row on the screen.
#[derive(Debug)]
struct Placed {
    line: u64,
    bytes: Range<usize>,
    /// The left edge of its text and its top.
    at: egui::Pos2,
}

/// The sizes of the pane in one frame.
struct Metrics {
    row: f32,
    column: f32,
    font: FontId,
    /// The left edge of the time and of the source.
    time: f32,
    source: f32,
    /// The name of each source in the gutter, for a view with more than one source. Empty
    /// for a view with one source: no line names its source.
    names: Vec<String>,
    /// Where the text goes.
    text: Rect,
    columns: usize,
    /// The count of whole rows in the pane.
    rows: usize,
    vertical: Rect,
    horizontal: Option<Rect>,
}

impl Default for Pane {
    fn default() -> Self {
        Self {
            top: Position { line: 0, row: 0 },
            offset: 0.0,
            follow: true,
            left: 0.0,
            selection: None,
            reveal: None,
            placed: Vec::new(),
            widest: 0.0,
        }
    }
}

impl Pane {
    /// Shows this line in the next frame, and ends follow.
    pub(crate) fn reveal(&mut self, line: u64) {
        (self.reveal, self.follow) = (Some(line), false);
    }

    /// The first line on the screen, for a search from the place of the pane.
    pub(crate) fn top_line(&self) -> u64 {
        self.top.line
    }

    /// Draws the pane into the rest of `ui`.
    pub(crate) fn show(&mut self, ui: &mut Ui, input: &Input<'_>) -> Output {
        // A wide part above can widen the `Ui`; the pane keeps to the part on the screen, or
        // its bar would be past the edge of the dock.
        let rect = ui.available_rect_before_wrap().intersect(ui.clip_rect());
        ui.advance_cursor_after_rect(rect);
        let id = ui.id().with("log pane");
        let response = ui.interact(rect, id, Sense::click_and_drag());
        let palette = theme::palette(ui);
        ui.painter_at(rect).rect_filled(rect, 0.0, palette.bg_sunken);
        let metrics = Metrics::of(ui, rect, input);
        let shown = input.shown();
        let cut = Cut { columns: input.wrap.then_some(metrics.columns) };
        if !input.wrap {
            self.left = self.left.clamp(0.0, (self.widest - metrics.text.width()).max(0.0));
        } else {
            self.left = 0.0;
        }
        let mut output = Output::default();
        let scrolled_down = self.input(ui, &response, input, cut, &metrics, &mut output);
        self.place(shown, cut, &metrics, scrolled_down);
        self.draw(ui, id, input, cut, &metrics, &mut output);
        self.bars(ui, id, shown, &metrics);
        output
    }

    /// The position of this frame: the end with follow, the line to reveal, or the
    /// position of before on a shown line, never after the last screen.
    fn place(&mut self, shown: Shown<'_>, cut: Cut, metrics: &Metrics, scrolled_down: bool) {
        let Some(end) = rows::end(shown, cut) else {
            self.offset = 0.0;
            return;
        };
        let last_top = rows::walk(shown, cut, end, -(metrics.rows.max(1) as isize - 1));
        if let Some(line) = self.reveal.take()
            && !self.placed.iter().any(|placed| placed.line == line)
        {
            let start = Position { line, row: 0 };
            (self.top, self.offset) = (rows::walk(shown, cut, start, -(metrics.rows as isize / 3)), 0.0);
        }
        self.top = rows::walk(shown, cut, self.top, 0);
        if self.follow || order(shown, self.top) >= order(shown, last_top) {
            (self.top, self.offset) = (last_top, 0.0);
            // A scroll down to the end starts follow again.
            self.follow |= scrolled_down;
        }
    }

    /// The bars: a drag of the vertical one goes to a line, and to the end starts follow.
    fn bars(&mut self, ui: &mut Ui, id: egui::Id, shown: Shown<'_>, metrics: &Metrics) {
        let total = shown.len() as f32;
        let visible = metrics.rows as f32;
        let start = shown.index_from(self.top.line) as f32;
        let bar = bars::Bar { rect: metrics.vertical, vertical: true, start, visible, total };
        if let Some(start) = bar.show(ui, id.with("vertical bar")) {
            let index = start.round() as usize;
            if let Some(line) = shown.number(index) {
                (self.top, self.offset) = (Position { line, row: 0 }, 0.0);
            }
            self.follow = start + visible >= total - 0.5;
            ui.ctx().request_repaint();
        }
        if let Some(rect) = metrics.horizontal {
            let bar = bars::Bar {
                rect,
                vertical: false,
                start: self.left,
                visible: metrics.text.width(),
                total: self.widest,
            };
            if let Some(start) = bar.show(ui, id.with("horizontal bar")) {
                self.left = start;
                ui.ctx().request_repaint();
            }
        }
    }
}

/// The place of a position in the sequence of the shown rows.
fn order(shown: Shown<'_>, position: Position) -> (usize, usize) {
    (shown.index_from(position.line), position.row)
}

impl Metrics {
    fn of(ui: &Ui, rect: Rect, input: &Input<'_>) -> Self {
        let font = theme::mono();
        let (row, column) = ui.ctx().fonts_mut(|fonts| (fonts.row_height(&font) + 3.0, fonts.glyph_width(&font, 'x')));
        let column = column.max(1.0);
        let (vertical, horizontal) = bars::places(rect, !input.wrap);
        let time = rect.left() + MARGIN;
        let source = if input.times { time + TIME_COLUMNS * column } else { time };
        let names = if input.view.sources.len() > 1 { draw::source_names(input.view) } else { Vec::new() };
        let gutter = draw::source_columns(input.view.request.kind, &names);
        let text_left = gutter.map_or(source, |columns| source + columns as f32 * column);
        let bottom = horizontal.map_or(rect.bottom(), |bar| bar.top());
        let text = Rect::from_min_max(pos2(text_left, rect.top() + 2.0), pos2(vertical.left() - MARGIN, bottom));
        let columns = ((text.width() / column).floor() as usize).max(8);
        let rows = ((text.height() / row).floor() as usize).max(1);
        Self { row, column, font, time, source, names, text, columns, rows, vertical, horizontal }
    }
}
