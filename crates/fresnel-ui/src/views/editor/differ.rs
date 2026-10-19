//! The rows of the diff of a save dialog (plan section 12.9). Myers' diff of 8,000 lines with
//! 1,000 changes takes longer than a frame, so the rows of large texts come from a worker
//! thread, and the dialog shows them in a later frame. The rows of small texts come in the
//! frame: a dialog of a usual manifest shows its diff at once.
//!
//! The editors share one worker, which starts with the first large diff: one dialog is open
//! at a time. A result carries the number of its request, and one for an other dialog has no
//! reader.

use std::sync::Arc;

use fresnel_core::manifest::EditText;
use fresnel_core::net::task::{TaskName, Tasks};

use super::diff::{self, Row};
use crate::widgets::worker::{Cancel, RequestId, Wake, Worker};

/// The most lines and bytes of the two texts whose diff the frame computes. The work of Myers'
/// diff grows with the lines, and each compare reads two lines up to their first difference:
/// 1,000 lines of 6,000 characters that a save changes at their end took 41 ms in a frame of a
/// release build, and 1,000 such lines in 128 KiB 3 ms. A usual manifest has some hundred
/// short lines.
const IN_FRAME: (usize, usize) = (1_000, 128 * 1024);

/// The two texts of a diff, and true for a Secret.
#[derive(Clone)]
struct Texts {
    old: Arc<str>,
    new: Arc<str>,
    secret: bool,
}

/// The rows of one dialog: computed, or the number of their request on the worker.
#[derive(Debug)]
pub(super) enum Rows {
    Ready(Vec<Row>),
    Waiting(u64),
}

/// The worker of the diffs of the editors.
pub(super) struct Differ {
    worker: Option<Worker<Texts, Vec<Row>>>,
    /// The number of the last request.
    asked: u64,
    tasks: Tasks,
    wake: Wake,
}

impl Differ {
    pub(super) fn new(tasks: &Tasks, wake: Wake) -> Self {
        Self { worker: None, asked: 0, tasks: tasks.clone(), wake }
    }

    /// The rows of the diff of `old` and `new`, of a Secret when `secret` is true: in this
    /// frame for small texts, else on the worker.
    pub(super) fn rows(&mut self, (old, new): (&EditText, &EditText), secret: bool) -> Rows {
        let (lines, bytes) = IN_FRAME;
        if old.lines + new.lines <= lines && old.text.len() + new.text.len() <= bytes {
            return Rows::Ready(diff::rows(&old.text, &new.text, secret));
        }
        self.asked += 1;
        let worker = self.worker.get_or_insert_with(|| {
            let compute = |texts: &Texts, _: &Cancel<'_>| Some(diff::rows(&texts.old, &texts.new, texts.secret));
            Worker::start(&self.tasks, TaskName::new("editor diff"), self.wake.clone(), compute)
        });
        let texts = Texts { old: Arc::clone(&old.text), new: Arc::clone(&new.text), secret };
        worker.submit(RequestId { query: self.asked, data: 0 }, texts);
        Rows::Waiting(self.asked)
    }

    /// Takes the rows of a dialog that waits for them, when the worker has them.
    pub(super) fn take(&self, rows: &mut Rows) {
        let (Rows::Waiting(number), Some(worker)) = (&*rows, &self.worker) else {
            return;
        };
        match worker.take() {
            Some(done) if done.id.query == *number => *rows = Rows::Ready(done.output),
            Some(done) => worker.discard(done.output),
            None => {}
        }
    }

    /// Empties the mailbox of the worker after a dialog that closed before its rows: they hold
    /// the lines of two texts.
    pub(super) fn forget(&self, rows: &Rows) {
        if let (Rows::Waiting(_), Some(worker)) = (rows, &self.worker) {
            worker.clear();
        }
    }

    /// Why the worker does not compute now, if it does not.
    pub(super) fn failure(&self) -> Option<String> {
        self.worker.as_ref().and_then(Worker::failure)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, channel};

    use super::*;
    use crate::testing::{LIMIT, TestTasks};

    /// A text of `lines` lines with a new line in place of each eighth one, `changed` times.
    fn text(lines: usize, changed: usize) -> EditText {
        let new = |number: usize| number.is_multiple_of(8) && number / 8 < changed;
        let line = |number: usize| if new(number) { "new\n".to_owned() } else { format!("{number}\n") };
        let text: String = (0..lines).map(line).collect();
        EditText { text: text.into(), version: None, lines, decoded: None }
    }

    /// Waits until the worker gives the rows.
    fn ready(differ: &Differ, woken: &Receiver<()>, rows: &mut Rows) {
        while matches!(rows, Rows::Waiting(_)) {
            woken.recv_timeout(LIMIT).expect("the rows of the worker");
            differ.take(rows);
        }
    }

    fn added(rows: &[Row]) -> usize {
        rows.iter().filter(|row| row.mark == diff::Mark::Added).count()
    }

    /// Small texts give their rows at once; large ones on the worker, and the result of an
    /// earlier dialog is no result of the next one.
    #[test]
    fn small_texts_give_their_rows_at_once_and_large_ones_from_the_worker() {
        let bed = TestTasks::new();
        let (sender, woken) = channel();
        let mut differ = Differ::new(&bed.tasks, Arc::new(move || if sender.send(()).is_err() {}));
        let (old, new) = (text(500, 0), text(500, 10));
        assert!(matches!(differ.rows((&old, &new), false), Rows::Ready(rows) if added(&rows) == 10));

        let (old, new) = (text(501, 0), text(501, 20));
        let mut first = differ.rows((&old, &new), false);
        differ.forget(&first);
        let mut second = differ.rows((&old, &new), false);
        ready(&differ, &woken, &mut second);
        differ.take(&mut first);
        assert!(matches!(first, Rows::Waiting(1)), "the rows of the first dialog went");
        assert!(matches!(second, Rows::Ready(rows) if added(&rows) == 20));

        // Few lines, but long ones: each compare reads them, so the worker computes them too.
        let long =
            |end: &str| EditText { text: format!("{}{end}\n", "a".repeat(70_000)).into(), lines: 1, ..text(0, 0) };
        let mut rows = differ.rows((&long("old"), &long("new")), false);
        assert!(matches!(rows, Rows::Waiting(3)), "two long lines");
        ready(&differ, &woken, &mut rows);
        assert!(matches!(rows, Rows::Ready(rows) if added(&rows) == 1));
    }
}
