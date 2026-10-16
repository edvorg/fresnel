//! The check of the text of an editor on a worker thread (plan section 12.9): the parse of
//! 8,000 lines takes some milliseconds, more than a frame can give at each key press.
//!
//! The editors share one worker, which starts with the first check. Only the tab on the
//! screen asks, and a result carries the identity of its request: a result for a text that
//! changed since, or for an other tab, has no reader. A text that is not checked yet keeps
//! the marks of its last check.

use std::sync::Arc;

use fresnel_core::manifest::check::{self, Check, Identity};
use fresnel_core::net::task::{TaskName, Tasks};

use crate::widgets::worker::{RequestId, Wake, Worker};

/// One text to check.
#[derive(Clone)]
struct Job {
    text: Arc<str>,
    identity: Identity,
}

/// The worker of the checks of the editors.
pub(super) struct Checker {
    worker: Option<Worker<Job, Check>>,
    /// The number of the last request.
    query: u64,
    tasks: Tasks,
    wake: Wake,
}

/// The check of one tab: the generation of the text that it asked for last, and the last
/// result with the generation of its text.
#[derive(Debug, Default)]
pub(super) struct TabCheck {
    asked: Option<(RequestId, u64)>,
    pub(super) done: Option<(u64, Arc<Check>)>,
}

impl TabCheck {
    /// True while the marks are of an older text.
    pub(super) fn is_behind(&self, generation: u64) -> bool {
        self.done.as_ref().is_none_or(|(done, _)| *done != generation)
    }
}

impl Checker {
    pub(super) fn new(tasks: &Tasks, wake: Wake) -> Self {
        Self { worker: None, query: 0, tasks: tasks.clone(), wake }
    }

    /// Asks for the check of a text of a tab when its last request was for an other
    /// generation, and takes the result of the worker. `text` gives the text only when the
    /// check needs it.
    pub(super) fn follow(
        &mut self,
        tab: (u64, &mut TabCheck),
        generation: u64,
        text: impl FnOnce() -> (String, Identity),
    ) {
        let (number, check) = tab;
        let worker = self.worker.get_or_insert_with(|| {
            let compute =
                |job: &Job, _: &crate::widgets::worker::Cancel<'_>| Some(check::check(&job.text, &job.identity));
            Worker::start(&self.tasks, TaskName::new("editor check"), self.wake.clone(), compute)
        });
        if let Some(done) = worker.take() {
            match check.asked {
                Some((id, asked)) if id == done.id => check.done = Some((asked, Arc::new(done.output))),
                _ => worker.discard(done.output),
            }
        }
        if check.asked.is_none_or(|(_, asked)| asked != generation) {
            self.query += 1;
            let id = RequestId { query: self.query, data: number };
            let (text, identity) = text();
            worker.submit(id, Job { text: text.into(), identity });
            check.asked = Some((id, generation));
        }
    }

    /// Why the worker does not check now, if it does not.
    pub(super) fn failure(&self) -> Option<String> {
        self.worker.as_ref().and_then(Worker::failure)
    }
}
