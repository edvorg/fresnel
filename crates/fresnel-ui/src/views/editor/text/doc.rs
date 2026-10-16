//! The text of an editor with its selection and its history, and each act of the user on
//! it: a key, a paste, a move, an undo. This part has no egui, so a unit test drives it.
//!
//! YAML allows no tab in an indent: the Tab key gives spaces to the next even column, or
//! two spaces before each selected line. Enter keeps the indent of the line, two more after
//! a colon. A change that would give the text more lines than its limit, or more than 8 MiB,
//! does not go in.

use super::buffer::{Buffer, Change, Pos, end_of};
use super::history::{History, Step};
use super::moves::{self, Motion, Selection};

/// The spaces of one step of indent.
const INDENT: usize = 2;
/// The most bytes that a change can give a text. A paste of a line of megabytes, such as a
/// minified file, passes the line limit, and each key press copies and checks the full text.
/// The API server takes a request of 3 MiB at most, so a larger text has no save.
const MOST_BYTES: usize = 8 * 1024 * 1024;

/// What the user does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Act {
    /// Typed characters: a step of the undo joins them.
    Type(String),
    /// A paste, or the text of an input method.
    Insert(String),
    Newline,
    Indent,
    Unindent,
    /// Backspace (`forward` false) or Delete, of a character, a word, or the rest of the line.
    Delete {
        forward: bool,
        unit: Unit,
    },
    Move {
        motion: Motion,
        /// True with Shift: the selection grows.
        extend: bool,
    },
    SelectAll,
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unit {
    Character,
    Word,
    Line,
}

/// The end of an act.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Nothing changed.
    None,
    /// The cursor or the selection moved.
    Moved,
    /// The text changed.
    Changed,
    /// The text cannot take the change: the words tell why.
    Refused(String),
}

/// The text, the selection and the history of one editor.
#[derive(Debug)]
pub(crate) struct Doc {
    pub(crate) buffer: Buffer,
    pub(crate) selection: Selection,
    /// The column that up and down keep.
    goal: Option<usize>,
    history: History,
    /// The most lines of the text.
    limit: usize,
    editable: bool,
}

impl Doc {
    pub(crate) fn new(text: &str, limit: usize, editable: bool) -> Self {
        let buffer = Buffer::new(text);
        Self { buffer, selection: Selection::default(), goal: None, history: History::default(), limit, editable }
    }

    pub(crate) fn editable(&self) -> bool {
        self.editable
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// The selected text.
    pub(crate) fn selected(&self) -> String {
        self.buffer.slice(self.selection.range())
    }

    /// Puts the cursor at a place, or moves the head of the selection there.
    pub(crate) fn place(&mut self, at: Pos, extend: bool) {
        let at = self.buffer.clamp(at);
        self.selection = if extend { Selection { head: at, ..self.selection } } else { Selection::at(at) };
        self.goal = None;
    }

    pub(crate) fn select(&mut self, selection: Selection) {
        let (anchor, head) = (self.buffer.clamp(selection.anchor), self.buffer.clamp(selection.head));
        self.selection = Selection { anchor, head };
        self.goal = None;
    }

    /// Runs one act. `now` is the time of the input, in seconds; `page` the lines of a page.
    pub(crate) fn act(&mut self, act: Act, now: f64, page: usize) -> Outcome {
        let changes = !matches!(act, Act::Move { .. } | Act::SelectAll);
        if changes && !self.editable {
            return Outcome::Refused("The text is read-only.".to_owned());
        }
        match act {
            Act::Type(text) => self.replace(self.selection.range(), &text, Some(now)),
            Act::Insert(text) => self.replace(self.selection.range(), &text.replace("\r\n", "\n"), None),
            Act::Newline => {
                let head = self.selection.range().start;
                let text = format!("\n{}", " ".repeat(next_indent(self.buffer.line(head.line), head.byte)));
                self.replace(self.selection.range(), &text, None)
            }
            Act::Indent if self.selection.range().start.line == self.selection.range().end.line => {
                let head = self.selection.range().start;
                let column = moves::column(self.buffer.line(head.line), head.byte);
                let spaces = INDENT - column % INDENT;
                self.replace(self.selection.range(), &" ".repeat(spaces), None)
            }
            Act::Indent => self.shift_lines(true),
            Act::Unindent => self.shift_lines(false),
            Act::Delete { forward, unit } => self.delete(forward, unit),
            Act::Move { motion, extend } => self.go(motion, extend, page),
            Act::SelectAll => {
                self.select(Selection { anchor: Pos::new(0, 0), head: self.buffer.end() });
                Outcome::Moved
            }
            Act::Undo => self.undo(),
            Act::Redo => self.redo(),
        }
    }

    fn go(&mut self, motion: Motion, extend: bool, page: usize) -> Outcome {
        let Selection { anchor, head } = self.selection;
        let vertical = matches!(motion, Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown);
        let goal = self.goal.unwrap_or_else(|| moves::column(self.buffer.line(head.line), head.byte));
        // With a selection, Left and Right go to its edge.
        let to = match motion {
            Motion::Left if !extend && anchor != head => anchor.min(head),
            Motion::Right if !extend && anchor != head => anchor.max(head),
            _ => moves::go(&self.buffer, head, motion, goal, page),
        };
        self.selection = if extend { Selection { anchor, head: to } } else { Selection::at(to) };
        self.goal = vertical.then_some(goal);
        Outcome::Moved
    }

    fn delete(&mut self, forward: bool, unit: Unit) -> Outcome {
        let range = self.selection.range();
        if !self.selection.is_empty() {
            return self.replace(range, "", None);
        }
        let head = range.start;
        let line = self.buffer.line(head.line);
        let before = line.get(..head.byte).unwrap_or_default();
        let other = match (forward, unit) {
            (false, Unit::Line) => Pos::new(head.line, 0),
            (true, Unit::Line) => Pos::new(head.line, line.len()),
            (false, Unit::Word) => moves::go(&self.buffer, head, Motion::WordLeft, 0, 0),
            (true, Unit::Word) => moves::go(&self.buffer, head, Motion::WordRight, 0, 0),
            // Backspace in an indent takes one step of indent.
            (false, Unit::Character) if !before.is_empty() && before.bytes().all(|byte| byte == b' ') => {
                Pos::new(head.line, head.byte - ((before.len() - 1) % INDENT + 1))
            }
            (false, Unit::Character) => moves::go(&self.buffer, head, Motion::Left, 0, 0),
            (true, Unit::Character) => moves::go(&self.buffer, head, Motion::Right, 0, 0),
        };
        if other == head {
            return Outcome::None;
        }
        self.replace(head.min(other)..head.max(other), "", None)
    }

    /// Adds or removes one step of indent before each line of the selection.
    fn shift_lines(&mut self, more: bool) -> Outcome {
        let Selection { anchor, head } = self.selection;
        let range = self.selection.range();
        // A selection that ends at the start of a line does not take that line.
        let last =
            if range.end.byte == 0 && range.end.line > range.start.line { range.end.line - 1 } else { range.end.line };
        let lines: Vec<String> = (range.start.line..=last).map(|line| self.buffer.line(line).to_owned()).collect();
        let mut shifts = Vec::with_capacity(lines.len());
        let new: Vec<String> = lines
            .iter()
            .map(|line| {
                if more {
                    shifts.push(INDENT as isize);
                    format!("{}{line}", " ".repeat(INDENT))
                } else {
                    let spaces = (line.len() - line.trim_start_matches(' ').len()).min(INDENT);
                    shifts.push(-(spaces as isize));
                    line.get(spaces..).unwrap_or_default().to_owned()
                }
            })
            .collect();
        if shifts.iter().all(|shift| *shift == 0) {
            return Outcome::None;
        }
        let end = Pos::new(last, lines.last().map_or(0, String::len));
        let shifted = |pos: Pos| match shifts.get(pos.line.wrapping_sub(range.start.line)) {
            Some(shift) if pos.line <= last => Pos::new(pos.line, pos.byte.saturating_add_signed(*shift)),
            _ => pos,
        };
        let after = Selection { anchor: shifted(anchor), head: shifted(head) };
        self.change(Pos::new(range.start.line, 0)..end, &new.join("\n"), after, None)
    }

    /// Replaces a range with a text, and puts the cursor after it.
    fn replace(&mut self, range: std::ops::Range<Pos>, text: &str, typed: Option<f64>) -> Outcome {
        let after = Selection::at(end_of(self.buffer.clamp(range.start), text));
        self.change(range, text, after, typed)
    }

    /// Applies a change with the selection after it, unless the text would get more lines
    /// than its limit, or would grow past `MOST_BYTES`.
    fn change(&mut self, range: std::ops::Range<Pos>, text: &str, after: Selection, typed: Option<f64>) -> Outcome {
        let (before, bytes) = (self.selection, self.buffer.bytes());
        let change = self.buffer.replace(range, text);
        let grown = self.buffer.bytes();
        let refusal = if self.buffer.text_lines() > self.limit {
            Some(format!("{} lines", crate::format::thousands(self.limit as u64)))
        } else {
            (grown > MOST_BYTES && grown > bytes).then(|| "8 MiB of text".to_owned())
        };
        if let Some(most) = refusal {
            self.buffer.apply(&change.reverse());
            return Outcome::Refused(format!("The editor holds {most} at most, so the text did not change."));
        }
        self.select(after);
        self.history.record(Step { change, before, after: self.selection }, typed);
        Outcome::Changed
    }

    fn undo(&mut self) -> Outcome {
        let Some(step) = self.history.undo() else {
            return Outcome::None;
        };
        self.apply(&step.change.reverse(), step.before)
    }

    fn redo(&mut self) -> Outcome {
        let Some(step) = self.history.redo() else {
            return Outcome::None;
        };
        self.apply(&step.change, step.after)
    }

    fn apply(&mut self, change: &Change, selection: Selection) -> Outcome {
        self.buffer.apply(change);
        self.select(selection);
        Outcome::Changed
    }
}

/// The indent of a new line after `byte` of `line`: the indent of the line, the place of
/// the text after a list dash, and one step more after a colon.
fn next_indent(line: &str, byte: usize) -> usize {
    let before = line.get(..byte).unwrap_or_default();
    let indent = before.len() - before.trim_start_matches(' ').len();
    let rest = before.get(indent..).unwrap_or_default();
    let dashes = rest.len() - rest.trim_start_matches(['-', ' ']).len();
    let item = if rest.starts_with("- ") { dashes } else { 0 };
    let colon = if before.trim_end().ends_with(':') { INDENT } else { 0 };
    indent + item + colon
}

#[cfg(test)]
mod tests;
