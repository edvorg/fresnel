//! The copy of a selection to the clipboard (plan section 12.7). A small selection becomes
//! its text in the frame; a large one on a worker thread, so that a copy of 200,000 lines
//! does not hold a frame. The clipboard gets the text in the frame after the worker.

use std::collections::VecDeque;
use std::sync::Arc;

use fresnel_core::logs::Lines;
use fresnel_core::net::task::{TaskName, Tasks};

use super::pane::rows::Shown;
use super::pane::select::{self, Columns, Selection};
use crate::format;
use crate::widgets::tone::Tone;
use crate::widgets::worker::{Cancel, RequestId, Wake, Worker};

/// The most lines that a frame copies itself: some hundred kilobytes at most.
const IN_FRAME: usize = 2_000;
/// The most bytes of one copy. A larger part of a log is a file: "Save".
const LIMIT: usize = 32 * 1024 * 1024;

/// One copy: the lines of the view, the lines that the filter keeps, the selection and the
/// columns of the pane.
#[derive(Clone)]
pub(super) struct Job {
    pub(super) lines: Lines,
    pub(super) kept: Option<Arc<VecDeque<u64>>>,
    pub(super) selection: Selection,
    pub(super) columns: Columns,
}

/// The text of a copy, its count of lines and true when it stopped at its limit.
type Copied = (String, u64, bool);

/// The copies of one view.
#[derive(Default)]
pub(super) struct Copier {
    worker: Option<Worker<Job, Copied>>,
    /// The number of the last copy.
    asked: u64,
    /// The count of lines of the copy on the worker.
    waiting: Option<usize>,
}

impl Copier {
    /// Copies a selection. Gives the note of a copy in the frame.
    pub(super) fn start(
        &mut self,
        ctx: &egui::Context,
        job: Job,
        tasks: &Tasks,
        wake: &Wake,
    ) -> Option<(Tone, String)> {
        let shown = Shown { lines: &job.lines, kept: job.kept.as_deref() };
        let (start, end) = job.selection.ordered();
        let count = shown.index_from(end.line.saturating_add(1)).saturating_sub(shown.index_from(start.line));
        self.asked += 1;
        if count <= IN_FRAME {
            self.waiting = None;
            return Some(done(ctx, select::text(shown, job.selection, &job.columns, LIMIT)));
        }
        let worker = self.worker.get_or_insert_with(|| {
            Worker::start(tasks, TaskName::new("log copy"), Arc::clone(wake), |job: &Job, _: &Cancel<'_>| {
                let shown = Shown { lines: &job.lines, kept: job.kept.as_deref() };
                Some(select::text(shown, job.selection, &job.columns, LIMIT))
            })
        });
        worker.submit(RequestId { query: self.asked, data: 0 }, job);
        self.waiting = Some(count);
        None
    }

    /// Puts the text of the worker on the clipboard. Gives its note.
    pub(super) fn poll(&mut self, ctx: &egui::Context) -> Option<(Tone, String)> {
        let worker = self.worker.as_ref()?;
        let copied = worker.take()?;
        if copied.id.query != self.asked {
            worker.discard(copied.output);
            return None;
        }
        self.waiting = None;
        Some(done(ctx, copied.output))
    }

    /// "Copying 120,000 lines…" while the worker builds the text, or why it stopped.
    pub(super) fn state(&self) -> Option<(Tone, String)> {
        let count = self.waiting?;
        if let Some(failure) = self.worker.as_ref().and_then(Worker::failure) {
            return Some((Tone::Error, format!("The copy stopped: {failure}")));
        }
        Some((Tone::Info, format!("Copying {}…", format::count(count as u64, "line"))))
    }
}

/// Puts the text on the clipboard and gives the note of the copy.
fn done(ctx: &egui::Context, (text, lines, cut): Copied) -> (Tone, String) {
    let size = format!("{}B", format::memory(text.len() as u64));
    let lines = format::count(lines, "line");
    ctx.copy_text(text);
    if cut {
        (Tone::Warn, format!("Copied {lines} ({size}), the most of one copy. Save the lines for the rest."))
    } else {
        (Tone::Info, format!("Copied {lines} ({size})."))
    }
}
