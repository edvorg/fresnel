//! The supply of rows of a table: the last request to the worker thread, the rows on the
//! screen, and how the two stand to each other.
//!
//! In the UI prototype, the table put each result of its worker into a state that had
//! changed after the request. Here one value holds the sort and the filter with the
//! identity of the last request, and it takes a result only if the two fit.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fresnel_core::net::task::{TaskName, Tasks};

use super::model::{Model, Sort};
use super::rows::{Request, Rows, sorted_rows};
use super::worker::{Cancel, Done, RequestId, Wake, Worker};

/// The table tells that it waits for new rows only after this time. The rows of 10,000
/// objects need some milliseconds, and a note for each of them would flicker.
pub(super) const SLOW: Duration = Duration::from_millis(250);

/// How the rows on the screen stand to the last request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Progress<'a> {
    /// The rows are the rows of the last request, or the newer ones arrive in a moment.
    Current,
    /// The worker computes newer rows, and that takes long.
    Behind,
    /// The worker did not give the rows of the last request, for this reason. The rows
    /// on the screen are older ones (design rules 1 and 5).
    Failed(&'a str),
}

/// The rows of one table. It owns the worker thread.
pub(super) struct Supply<M: Model> {
    worker: Worker<Request<M>, Rows<M>>,
    sort: Sort,
    /// The filter of the last request.
    filter: Option<M::Filter>,
    /// A number that grows with each change of the sort or of the filter.
    query: u64,
    /// The identity and the objects of the last request.
    asked: Option<(RequestId, M::Data)>,
    /// The rows on the screen with the identity of their request. `None` before the
    /// first result.
    shown: Option<Done<Rows<M>>>,
    /// Why the worker did not compute. It stays to the next result.
    failure: Option<String>,
    /// The moment from which the rows on the screen are not the rows of the last request.
    behind_since: Option<Instant>,
}

impl<M: Model> Supply<M> {
    /// Starts the worker thread. `wake` asks for a repaint for a result, and `name` names
    /// the thread.
    pub(super) fn start(tasks: &Tasks, wake: Wake, name: &'static str, model: Arc<M>) -> Self {
        let compute = move |request: &Request<M>, cancel: &Cancel<'_>| sorted_rows(&*model, request, cancel);
        let worker = Worker::start(tasks, TaskName::scoped("table rows", name), wake, compute);
        Self {
            worker,
            sort: Sort { column: 0, descending: false },
            filter: None,
            query: 0,
            asked: None,
            shown: None,
            failure: None,
            behind_since: None,
        }
    }

    pub(super) fn sort(&self) -> Sort {
        self.sort
    }

    /// Sets the sort. The next request asks the worker for the rows in the new sequence.
    pub(super) fn set_sort(&mut self, sort: Sort) {
        self.sort = sort;
        self.query += 1;
    }

    /// Tells what the rows come from in this frame, and takes a result of the worker.
    /// `data_serial` grows when the objects change, and `data` gives them.
    pub(super) fn request(&mut self, filter: &M::Filter, data_serial: u64, data: impl FnOnce() -> M::Data) {
        if self.filter.as_ref() != Some(filter) {
            self.filter = Some(filter.clone());
            self.query += 1;
        }
        let id = RequestId { query: self.query, data: data_serial };
        if self.asked_id() != Some(id) {
            let data = data();
            self.worker.submit(id, Request { sort: self.sort, filter: filter.clone(), data: data.clone() });
            self.asked = Some((id, data));
        }
        self.take_result();
    }

    /// Takes the newest result. A result of an older sort or filter does not fit the
    /// header and the filter that the user sees now, so the table drops it. A result of
    /// this query for older objects fits: its rows are newer than the rows on the screen.
    fn take_result(&mut self) {
        if let Some(done) = self.worker.take() {
            let old = if done.id.query == self.query {
                self.failure = None;
                self.shown.replace(done)
            } else {
                Some(done)
            };
            // The thread frees the rows: they can hold the last pointer to many objects.
            old.into_iter().for_each(|old| self.worker.discard(old.output));
        }
        if !self.is_behind() {
            self.behind_since = None;
            return;
        }
        self.behind_since.get_or_insert_with(Instant::now);
        if let Some(failure) = self.worker.failure() {
            self.failure = Some(failure);
        }
    }

    fn asked_id(&self) -> Option<RequestId> {
        self.asked.as_ref().map(|(id, _)| *id)
    }

    /// True while the rows on the screen are not the rows of the last request.
    fn is_behind(&self) -> bool {
        self.asked_id() != self.shown.as_ref().map(|done| done.id)
    }

    /// The rows on the screen with the objects of the last request, while the rows are
    /// older than these objects: the worker computes, and the frames go on. A row can
    /// then be the row of an object that went away.
    pub(super) fn older_rows(&self) -> Option<(&[M::Row], &M::Data)> {
        let (asked, objects) = self.asked.as_ref()?;
        let shown = self.shown.as_ref().filter(|shown| shown.id != *asked)?;
        Some((&shown.output.rows, objects))
    }

    pub(super) fn progress(&self) -> Progress<'_> {
        match &self.failure {
            Some(failure) => Progress::Failed(failure),
            None if self.behind_since.is_some_and(|since| since.elapsed() >= SLOW) => Progress::Behind,
            None => Progress::Current,
        }
    }

    /// Why the worker did not compute the rows of the last request.
    pub(super) fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// The time to the frame that tells that the table waits for its rows. `None` when
    /// no such frame comes.
    pub(super) fn time_to_slow(&self) -> Option<Duration> {
        SLOW.checked_sub(self.behind_since?.elapsed())
    }

    /// Gives up the rows and the objects of the last request, for a table that is not
    /// painted (plan section 7.3). The worker thread frees the rows, and the next request
    /// asks for rows again. A call with nothing to give up does nothing.
    pub(super) fn rest(&mut self) {
        if self.asked.is_none() && self.shown.is_none() {
            return;
        }
        self.worker.clear();
        if let Some(shown) = self.shown.take() {
            self.worker.discard(shown.output);
        }
        (self.asked, self.failure, self.behind_since) = (None, None, None);
    }

    /// The rows on the screen. `None` before the first result.
    pub(super) fn rows(&self) -> Option<&Rows<M>> {
        self.shown.as_ref().map(|done| &done.output)
    }

    /// The time that the worker thread took for the rows on the screen.
    pub(super) fn compute_time(&self) -> Option<Duration> {
        self.shown.as_ref().map(|done| done.time)
    }
}
