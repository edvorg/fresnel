//! The check of the text of an editor on a worker thread (plan section 12.9): the parse of
//! 8,000 lines takes some milliseconds, more than a frame can give at each key press.
//!
//! The editors share one worker, which starts with the first check. Only the tab on the
//! screen asks, and a result carries the identity of its request: a result for a text that
//! changed since, or for an other tab, has no reader. The mailbox of the worker holds one
//! request, so the request of a tab takes the place of the request of an other tab, or takes
//! its result: that tab asks again when it shows. A text that is not checked yet keeps the
//! marks of its last check.

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
    /// The last request: each one before it has no result.
    last: Option<RequestId>,
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
        Self { worker: None, query: 0, last: None, tasks: tasks.clone(), wake }
    }

    /// Takes the result of the worker, and asks for the check of a text of a tab whose marks
    /// are of an other generation, unless the last request is for this one. `text` gives the
    /// text only when the check needs it.
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
        let waits = check.asked.is_some_and(|(id, asked)| asked == generation && self.last == Some(id));
        if check.is_behind(generation) && !waits {
            self.query += 1;
            let id = RequestId { query: self.query, data: number };
            let (text, identity) = text();
            worker.submit(id, Job { text: text.into(), identity });
            (check.asked, self.last) = (Some((id, generation)), Some(id));
        }
    }

    /// Why the worker does not check now, if it does not.
    pub(super) fn failure(&self) -> Option<String> {
        self.worker.as_ref().and_then(Worker::failure)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, channel};

    use fresnel_core::kinds::Kind;
    use fresnel_core::store::ObjectKey;

    use super::*;
    use crate::testing::{LIMIT, TestTasks};

    /// Follows a tab until it has the check of its text.
    fn checked(checker: &mut Checker, woken: &Receiver<()>, tab: (u64, &mut TabCheck), job: &(String, Identity)) {
        let (number, check) = tab;
        checker.follow((number, &mut *check), 1, || job.clone());
        while check.is_behind(1) {
            woken.recv_timeout(LIMIT).expect("a result of the worker");
            checker.follow((number, &mut *check), 1, || job.clone());
        }
    }

    /// The second tab shows while the worker checks the text of the first one: its request
    /// takes the place of the first one, or its result. The first tab asks again when it
    /// shows, and gets its check.
    #[test]
    fn a_tab_whose_request_an_other_tab_took_asks_again() {
        let bed = TestTasks::new();
        let (sender, woken) = channel();
        let mut checker = Checker::new(&bed.tasks, Arc::new(move || if sender.send(()).is_err() {}));
        let identity = Identity { kind: Kind::Pod, key: ObjectKey::new("shop", "web") };
        let valid = ("kind: Pod\nmetadata: {name: web, namespace: shop}\n".to_owned(), identity.clone());
        let invalid = ("kind: [Pod\n".to_owned(), identity);
        let (mut first, mut second) = (TabCheck::default(), TabCheck::default());
        checker.follow((1, &mut first), 1, || valid.clone());
        checked(&mut checker, &woken, (2, &mut second), &invalid);
        checked(&mut checker, &woken, (1, &mut first), &valid);
        let errors = |check: &TabCheck| check.done.as_ref().map(|(_, done)| done.has_error());
        assert_eq!((errors(&first), errors(&second)), (Some(false), Some(true)), "each tab has the check of its text");
    }
}
