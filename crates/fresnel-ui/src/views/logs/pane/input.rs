//! The input of the pane: the wheel, the keys and the pointer. A key acts while the pane
//! has the focus, which a click on it gives.

use egui::{Event, Key, Pos2, Response, Ui};

use super::rows::{self, Cut, Position, Shown};
use super::select::{Point, Selection};
use super::{Input, Metrics, Output, Pane};

impl Pane {
    /// Takes the input of this frame. Gives true when the user moved down: at the end, that
    /// starts follow again.
    pub(super) fn input(
        &mut self,
        ui: &Ui,
        response: &Response,
        input: &Input<'_>,
        cut: Cut,
        metrics: &Metrics,
        output: &mut Output,
    ) -> bool {
        let shown = input.shown();
        let mut moves = Vec::new();
        if response.hovered() {
            let delta = ui.input_mut(|input| std::mem::take(&mut input.smooth_scroll_delta));
            moves.push(-delta.y);
            self.left = (self.left - delta.x).max(0.0);
        }
        if response.has_focus() {
            moves.extend(self.keys(ui, shown, metrics, output));
        }
        moves.extend(self.pointer(ui, response, shown, metrics));
        for points in &moves {
            self.scroll(shown, cut, metrics.row, *points);
        }
        moves.iter().any(|points| *points > 0.0)
    }

    /// Moves the rows by `points`: a positive count moves to later lines, a negative one
    /// ends follow.
    fn scroll(&mut self, shown: Shown<'_>, cut: Cut, row: f32, points: f32) {
        if points == 0.0 {
            return;
        }
        if points < 0.0 {
            self.follow = false;
        }
        let mut offset = self.offset + points;
        while offset < 0.0 {
            let up = rows::walk(shown, cut, self.top, -1);
            if up == self.top {
                offset = 0.0;
                break;
            }
            (self.top, offset) = (up, offset + row);
        }
        while offset >= row {
            let down = rows::walk(shown, cut, self.top, 1);
            if down == self.top {
                offset = 0.0;
                break;
            }
            (self.top, offset) = (down, offset - row);
        }
        self.offset = offset;
    }

    /// The keys of the pane: the arrows, the pages, the start and the end, select all and
    /// copy. Gives the moves of the rows, in points.
    fn keys(&mut self, ui: &Ui, shown: Shown<'_>, metrics: &Metrics, output: &mut Output) -> Vec<f32> {
        let page = metrics.rows.saturating_sub(1).max(1) as f32 * metrics.row;
        let events = ui.input(|input| input.events.clone());
        let mut moves = Vec::new();
        for event in events {
            match event {
                Event::Copy => output.copy = self.selection.is_some_and(|selection| !selection.is_empty()),
                Event::Key { key, pressed: true, modifiers, .. } => match key {
                    Key::ArrowUp => moves.push(-metrics.row),
                    Key::ArrowDown => moves.push(metrics.row),
                    Key::PageUp => moves.push(-page),
                    Key::PageDown => moves.push(page),
                    Key::Home => {
                        let first = shown.number(0).map(|line| Position { line, row: 0 });
                        (self.top, self.offset, self.follow) = (first.unwrap_or(self.top), 0.0, false);
                    }
                    Key::End => self.follow = true,
                    Key::A if modifiers.command => self.select_all(shown),
                    _ => {}
                },
                _ => {}
            }
        }
        moves
    }

    /// Selects each shown line.
    fn select_all(&mut self, shown: Shown<'_>) {
        let first = shown.number(0);
        let last = shown.len().checked_sub(1).and_then(|index| shown.line(index));
        if let (Some(first), Some((number, line))) = (first, last) {
            let anchor = Point { line: first, byte: 0 };
            self.selection = Some(Selection { anchor, cursor: Point { line: number, byte: line.text.len() } });
        }
    }

    /// A press and a drag select from the place of the press. A drag past an edge moves the
    /// rows one row in each frame. Shift and a click move the end of the selection, and a
    /// double or triple click selects one line. Gives the moves of the rows, in points.
    fn pointer(&mut self, ui: &Ui, response: &Response, shown: Shown<'_>, metrics: &Metrics) -> Option<f32> {
        if response.drag_started() {
            response.request_focus();
            let origin = ui.input(|input| input.pointer.press_origin());
            if let Some(point) = origin.and_then(|origin| self.point_at(origin, shown, metrics)) {
                self.selection = Some(Selection { anchor: point, cursor: point });
                self.follow = false;
            }
        }
        let mut moved = None;
        if response.dragged()
            && let Some(pos) = response.interact_pointer_pos()
        {
            if pos.y < metrics.text.top() {
                moved = Some(-metrics.row);
            } else if pos.y > metrics.text.bottom() {
                moved = Some(metrics.row);
            }
            if let Some(point) = self.point_at(pos, shown, metrics)
                && let Some(selection) = &mut self.selection
            {
                selection.cursor = point;
            }
            // Past an edge, the rows move in each frame while the pointer stays.
            if moved.is_some() {
                ui.ctx().request_repaint();
            }
        }
        if response.clicked() {
            response.request_focus();
            let point = response.interact_pointer_pos().and_then(|pos| self.point_at(pos, shown, metrics));
            let shift = ui.input(|input| input.modifiers.shift);
            self.selection = match (self.selection, point) {
                (Some(selection), Some(point)) if shift => Some(Selection { cursor: point, ..selection }),
                _ => None,
            };
        }
        // egui counts a third click soon after two as a triple click: it selects the line too.
        if (response.double_clicked() || response.triple_clicked())
            && let Some(point) = response.interact_pointer_pos().and_then(|pos| self.point_at(pos, shown, metrics))
        {
            let (line, len) = (point.line, shown.lines.get(point.line).map_or(0, |line| line.text.len()));
            self.selection = Some(Selection { anchor: Point { line, byte: 0 }, cursor: Point { line, byte: len } });
        }
        moved
    }

    /// The place in the text under a point of the screen, by the rows of the last frame: a
    /// point above or below them is the start of the first row or the end of the last one.
    fn point_at(&self, pos: Pos2, shown: Shown<'_>, metrics: &Metrics) -> Option<Point> {
        let first = self.placed.first()?;
        if pos.y < first.at.y {
            return Some(Point { line: first.line, byte: first.bytes.start });
        }
        let index = self.placed.iter().rposition(|placed| placed.at.y <= pos.y)?;
        let placed = self.placed.get(index)?;
        if index + 1 == self.placed.len() && pos.y >= placed.at.y + metrics.row {
            return Some(Point { line: placed.line, byte: placed.bytes.end });
        }
        let column = ((pos.x - placed.at.x) / metrics.column).round().max(0.0) as usize;
        let text = shown.lines.get(placed.line)?.text.get(placed.bytes.clone())?;
        let byte = text.char_indices().nth(column).map_or(text.len(), |(byte, _)| byte);
        Some(Point { line: placed.line, byte: placed.bytes.start + byte })
    }
}
