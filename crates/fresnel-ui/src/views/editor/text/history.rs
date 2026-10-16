//! The undo and the redo of an editor: a list of changes, not of copies of the text.
//!
//! The undo of the text widget of egui keeps a copy of the full text for each step, so 100
//! steps of a manifest of 300 KB hold 30 MB (research 05 section 6.9). A step here holds the
//! text of its change only. Characters that the user types in one place, with less than a
//! second between them, are one step, as in other editors. The history has a limit of steps
//! and of bytes (design rule 12): the oldest steps leave it.

use std::collections::VecDeque;

use super::buffer::Change;
use super::moves::Selection;

/// The most steps of the undo.
const MOST_STEPS: usize = 1000;
/// The most bytes of the texts of the steps.
const MOST_BYTES: usize = 16 * 1024 * 1024;
/// The longest pause between two typed characters of one step, in seconds.
const TYPING_PAUSE: f64 = 1.0;

/// One step: a change with the selection before and after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    pub(crate) change: Change,
    pub(crate) before: Selection,
    pub(crate) after: Selection,
}

impl Step {
    fn bytes(&self) -> usize {
        self.change.removed.len() + self.change.inserted.len()
    }
}

#[derive(Debug, Default)]
pub(crate) struct History {
    done: VecDeque<Step>,
    undone: Vec<Step>,
    bytes: usize,
    /// The time of the last typed character, while the last step is typing.
    typing: Option<f64>,
}

impl History {
    /// Keeps a step. `typed` is the time of a typed character: such a step joins the step of
    /// the character before, in the same place.
    pub(crate) fn record(&mut self, step: Step, typed: Option<f64>) {
        self.undone.clear();
        let joins = |last: &Step, now: f64, then: f64| {
            now - then < TYPING_PAUSE && last.after.head == step.change.at && step.change.removed.is_empty()
        };
        if let (Some(now), Some(then), Some(last)) = (typed, self.typing, self.done.back_mut())
            && joins(last, now, then)
        {
            last.change.inserted.push_str(&step.change.inserted);
            last.after = step.after;
            self.bytes += step.change.inserted.len();
        } else {
            self.bytes += step.bytes();
            self.done.push_back(step);
        }
        self.typing = typed;
        while self.done.len() > MOST_STEPS || (self.bytes > MOST_BYTES && self.done.len() > 1) {
            if let Some(old) = self.done.pop_front() {
                self.bytes -= old.bytes();
            }
        }
    }

    /// The step to take back: the caller applies the reverse of its change.
    pub(crate) fn undo(&mut self) -> Option<Step> {
        let step = self.done.pop_back()?;
        self.bytes -= step.bytes();
        self.undone.push(step.clone());
        self.typing = None;
        Some(step)
    }

    /// The step to do again: the caller applies its change.
    pub(crate) fn redo(&mut self) -> Option<Step> {
        let step = self.undone.pop()?;
        self.bytes += step.bytes();
        self.done.push_back(step.clone());
        self.typing = None;
        Some(step)
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::editor::text::buffer::Pos;

    fn typed(at: Pos, text: &str) -> Step {
        let change = Change { at, removed: String::new(), inserted: text.to_owned() };
        let after = Selection::at(Pos::new(at.line, at.byte + text.len()));
        Step { change, before: Selection::at(at), after }
    }

    #[test]
    fn characters_typed_in_one_place_without_a_pause_are_one_step() {
        let mut history = History::default();
        history.record(typed(Pos::new(0, 0), "a"), Some(10.0));
        history.record(typed(Pos::new(0, 1), "b"), Some(10.5));
        history.record(typed(Pos::new(0, 2), "c"), Some(12.0));
        history.record(typed(Pos::new(3, 0), "d"), Some(12.1));
        let step = history.undo().expect("a step");
        assert_eq!(step.change.inserted, "d", "an other place");
        assert_eq!(history.undo().map(|step| step.change.inserted), Some("c".to_owned()), "after a pause");
        let joined = history.undo().expect("a step");
        assert_eq!((joined.change.inserted.as_str(), joined.after.head), ("ab", Pos::new(0, 2)));
        assert!(!history.can_undo() && history.can_redo());
        assert_eq!(history.redo().map(|step| step.change.inserted), Some("ab".to_owned()));
        history.record(typed(Pos::new(0, 2), "x"), None);
        assert!(!history.can_redo(), "a new change ends the redo");
    }

    #[test]
    fn the_oldest_steps_leave_a_history_at_its_limit() {
        let mut history = History::default();
        for number in 0..MOST_STEPS + 5 {
            history.record(typed(Pos::new(number, 0), "x"), None);
        }
        assert_eq!(history.done.len(), MOST_STEPS);
        assert_eq!(history.done.front().map(|step| step.change.at.line), Some(5));
        let big = "y".repeat(MOST_BYTES / 2 + 1);
        history.record(typed(Pos::new(0, 0), &big), None);
        history.record(typed(Pos::new(0, 0), &big), None);
        assert!(history.bytes <= MOST_BYTES && history.done.len() == 1, "{} steps", history.done.len());
    }
}
