//! The text widget of the editor (plan section 12.9): YAML with syntax colors, line numbers,
//! the marks of the check and the hits of a search, a selection, the keys of a code editor,
//! and an undo.
//!
//! The text widget of egui lays out its full text at each frame and builds a node of the
//! accessibility tree for each of its rows: with 8,000 lines, a frame took 8 to 11 ms with
//! no input and up to 47 ms after a key press (plan section 17, risk 5). This widget has its
//! own virtual rows: a frame lays out the lines on the screen only, one galley for each,
//! which egui keeps while the line does not change. A key press costs the same with 20 or
//! 8,000 lines. A line of megabytes, such as a value of a ConfigMap, shows the part around
//! the screen.
//!
//! - `buffer`: the lines and their change. `history`: undo and redo. `moves`: the cursor.
//! - `doc`: the acts of the user on the text. `input`: the events of egui as acts.
//! - `lexer`: the syntax colors of a line. `colors`: their values. `paint`: the rows.

mod buffer;
mod colors;
mod doc;
mod history;
mod input;
mod lexer;
mod moves;
mod paint;

use std::ops::Range;

use egui::{CursorIcon, EventFilter, Rect, ScrollArea, Sense, Ui, vec2};
use fresnel_core::manifest::check::Mark;

pub(crate) use buffer::Pos;
pub(crate) use doc::Act;
use doc::{Doc, Outcome};
use input::Input;
use lexer::State;
use moves::{Motion, Selection};
use paint::{Metrics, Row};

/// The name of the text area in the accessibility tree.
pub(crate) const LABEL: &str = "The text of the editor";

/// A place of a hit of the search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hit {
    pub(crate) line: usize,
    pub(crate) bytes: Range<usize>,
}

/// What the text shows besides itself: the marks of the check and the hits of a search.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Overlay<'a> {
    pub(crate) marks: &'a [Mark],
    pub(crate) hits: &'a [Hit],
    /// The index of the hit of the cursor.
    pub(crate) current: Option<usize>,
}

/// The widget of one editor, with its text.
#[derive(Debug)]
pub(crate) struct TextArea {
    doc: Doc,
    /// The state at the start of each line, for the generation of the text.
    states: (u64, Vec<State>),
    /// The most characters of a line, for the generation of the text.
    widest: (u64, usize),
    /// True when the next frame scrolls the cursor into the view.
    reveal: bool,
    /// The count of lines on the screen in the last frame: the step of Page Up and Page Down.
    page: usize,
    /// True when the area takes the focus in the next frame.
    focus: bool,
    /// Why the last change did not go in.
    note: Option<String>,
    /// True when the area had the focus of the keys in its last frame.
    focused: bool,
    /// The height of the area in its last frame.
    height: f32,
}

impl TextArea {
    /// A widget of a text with at most `limit` lines. A read-only text keeps its selection and
    /// its search.
    pub(crate) fn new(text: &str, limit: usize, editable: bool) -> Self {
        let doc = Doc::new(text, limit, editable);
        Self {
            doc,
            states: (u64::MAX, Vec::new()),
            widest: (u64::MAX, 0),
            reveal: false,
            page: 20,
            focus: false,
            note: None,
            focused: false,
            height: 0.0,
        }
    }

    pub(crate) fn text(&self) -> String {
        self.doc.buffer.text()
    }

    /// A number that grows with each change of the text.
    pub(crate) fn generation(&self) -> u64 {
        self.doc.buffer.generation()
    }

    /// The count of lines as a user counts them.
    pub(crate) fn lines(&self) -> usize {
        self.doc.buffer.text_lines()
    }

    /// The line with this number, from 0.
    pub(crate) fn line(&self, line: usize) -> &str {
        self.doc.buffer.line(line)
    }

    pub(crate) fn editable(&self) -> bool {
        self.doc.editable()
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.doc.can_undo()
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.doc.can_redo()
    }

    /// The place of the cursor.
    pub(crate) fn cursor(&self) -> Pos {
        self.doc.selection.head
    }

    /// The line and the column of the cursor, from 1, in characters, as the marks of the
    /// check count them.
    pub(crate) fn cursor_place(&self) -> (usize, usize) {
        let head = self.doc.selection.head;
        (head.line + 1, moves::column(self.doc.buffer.line(head.line), head.byte) + 1)
    }

    /// Why the last change did not go in.
    pub(crate) fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Runs an act of a button: an undo, a redo.
    pub(crate) fn act(&mut self, act: Act, now: f64) {
        let outcome = self.doc.act(act, now, self.page);
        self.end(outcome);
        self.focus = true;
    }

    /// Holds the text while a save of it is on its way, or ends the hold.
    pub(crate) fn hold(&mut self, held: bool) {
        self.doc.held = held;
    }

    /// Selects a range and shows it: a hit of the search, the place of a mark.
    pub(crate) fn select(&mut self, range: Range<Pos>) {
        self.doc.select(Selection { anchor: range.start, head: range.end });
        self.reveal = true;
    }

    /// Gives the area the focus of the keys in the next frame.
    pub(crate) fn focus(&mut self) {
        self.focus = true;
    }

    /// True when the area had the focus of the keys in its last frame.
    pub(crate) fn has_focus(&self) -> bool {
        self.focused
    }

    /// Draws the text into the rest of `ui`.
    pub(crate) fn show(&mut self, ui: &mut Ui, overlay: Overlay<'_>) {
        let id = ui.id().with("editor text");
        let focused = ui.memory(|memory| memory.has_focus(id));
        if focused {
            let filter = EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false };
            ui.memory_mut(|memory| memory.set_focus_lock_filter(id, filter));
            self.keys(ui);
        }
        self.follow();
        let metrics = Metrics::of(ui, self.doc.buffer.count());
        let palette = crate::theme::palette(ui);
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, palette.bg_sunken);
        // The list of the marks comes and goes below the area: the cursor stays in view.
        if self.focused && (rect.height() - std::mem::replace(&mut self.height, rect.height())).abs() > 0.5 {
            self.reveal = true;
        }
        let content = vec2(metrics.text_left + (self.widest.1 + 8) as f32 * metrics.column, metrics.height());
        let area = ScrollArea::both().id_salt("editor scroll").auto_shrink(false);
        area.show_viewport(ui, |ui, viewport| {
            let origin = ui.min_rect().min;
            ui.allocate_space(content);
            let visible = viewport.translate(origin.to_vec2());
            let response = ui.interact(visible, id, Sense::click_and_drag());
            ui.ctx().accesskit_node_builder(id, |node| {
                node.set_role(egui::accesskit::Role::MultilineTextInput);
                node.set_label(LABEL);
            });
            if response.contains_pointer() {
                ui.ctx().set_cursor_icon(CursorIcon::Text);
            }
            let rows = paint::rows(ui, &metrics, (origin, viewport), &self.doc.buffer, &self.states.1);
            self.page = rows.len().saturating_sub(1).max(1);
            self.pointer(ui, &response, &metrics, (origin, viewport), &rows);
            if std::mem::take(&mut self.focus) {
                response.request_focus();
            }
            let focused = response.has_focus();
            self.focused = focused;
            let cursor = paint::Cursor { selection: self.doc.selection, focused };
            paint::draw(ui, &metrics, (origin, viewport), &rows, &self.doc.buffer, (cursor, overlay));
            paint::gutter(ui, &metrics, (origin, viewport), &rows, overlay.marks, self.doc.selection.head.line);
            let head = self.doc.selection.head;
            let x = paint::x_of(&self.doc.buffer, &rows, &metrics, head);
            let head = Rect::from_min_size(
                origin + vec2(metrics.text_left + x, head.line as f32 * metrics.row),
                vec2(1.0, metrics.row),
            );
            // The cursor shows at once: a scroll that follows a key press with an animation
            // would show the text of before under the keys of the user.
            if std::mem::take(&mut self.reveal) {
                let around = head.expand2(vec2(metrics.column * 4.0, 0.0));
                ui.scroll_to_rect_animation(around, None, egui::style::ScrollAnimation::none());
            }
            if focused {
                ime(ui, visible, head);
            }
        });
    }

    /// Runs the events of the frame on the focused area.
    fn keys(&mut self, ui: &Ui) {
        let mac = ui.ctx().os() == egui::os::OperatingSystem::Mac;
        let (events, now) = ui.input(|input| (input.events.clone(), input.time));
        for event in &events {
            match input::of(event, mac) {
                Some(Input::Act(act)) => {
                    let outcome = self.doc.act(act, now, self.page);
                    self.end(outcome);
                }
                Some(Input::Copy) if !self.doc.selection.is_empty() => ui.ctx().copy_text(self.doc.selected()),
                Some(Input::Cut) if !self.doc.selection.is_empty() => {
                    ui.ctx().copy_text(self.doc.selected());
                    let outcome =
                        self.doc.act(Act::Delete { forward: false, unit: doc::Unit::Character }, now, self.page);
                    self.end(outcome);
                }
                _ => {}
            }
        }
    }

    /// Keeps the outcome of an act: a change or a move shows the cursor.
    fn end(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::None => {}
            Outcome::Moved => self.reveal = true,
            Outcome::Changed => (self.reveal, self.note) = (true, None),
            Outcome::Refused(note) => self.note = Some(note),
        }
    }

    /// The states of the lines and the widest line after a change of the text.
    fn follow(&mut self) {
        let generation = self.generation();
        if self.states.0 != generation {
            self.states = (generation, lexer::states(&self.doc.buffer));
        }
        if self.widest.0 != generation {
            let buffer = &self.doc.buffer;
            let widest = (0..buffer.count()).map(|line| buffer.line(line).chars().count()).max().unwrap_or(0);
            self.widest = (generation, widest);
        }
    }

    /// A press puts the cursor, a drag moves its head, a double click selects a word and a
    /// triple click a line. Shift with a press grows the selection.
    fn pointer(
        &mut self,
        ui: &Ui,
        response: &egui::Response,
        metrics: &Metrics,
        view: (egui::Pos2, Rect),
        rows: &[Row],
    ) {
        let Some(pointer) = response.interact_pointer_pos() else {
            return;
        };
        let at = paint::pos_at(ui, metrics, view, rows, &self.doc.buffer, (&self.states.1, pointer));
        let (pressed, shift) = ui.input(|input| (input.pointer.any_pressed(), input.modifiers.shift));
        if response.triple_clicked() {
            let line = Pos::new(at.line, 0);
            self.doc.select(Selection { anchor: line, head: moves::go(&self.doc.buffer, line, Motion::Down, 0, 0) });
        } else if response.double_clicked() {
            let word = moves::word_at(&self.doc.buffer, at);
            self.doc.select(Selection { anchor: word.start, head: word.end });
        } else if pressed && response.is_pointer_button_down_on() {
            self.doc.place(at, shift);
            response.request_focus();
        } else if response.dragged() {
            self.doc.place(at, true);
            self.reveal = true;
        }
    }
}

/// Tells the system where the cursor is, so the window of an input method shows there.
fn ime(ui: &Ui, rect: Rect, cursor_rect: Rect) {
    ui.output_mut(|output| {
        output.ime = Some(egui::output::IMEOutput {
            purpose: egui::IMEPurpose::Normal,
            rect,
            cursor_rect,
            should_interrupt_composition: false,
        });
    });
}

#[cfg(test)]
mod tests;
