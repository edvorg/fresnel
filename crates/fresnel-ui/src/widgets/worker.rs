//! A worker thread of a view: the rows of a table, the results of the quick search. It
//! computes them, so a sort, a filter or a search of 10,000 objects never holds a frame
//! (plan section 7.3).
//!
//! The UI thread puts a request into a mailbox with one place and reads the result in a
//! later frame. The rules come from the defects of the UI prototype:
//!
//! - New data does not stop the computation in progress. Only a new query does: with a
//!   stream of snapshots that is faster than the computation, each stop would mean that
//!   the table gets no rows at all.
//! - Each result carries the identity of its request, and the reader decides if it fits.
//! - The thread is a task with a state. After a panic it starts again and computes the
//!   request that waits: a request stays in the mailbox until its result is there.
//! - The mailbox holds one request and one result, because each request holds a snapshot
//!   of each store.
//! - The thread frees the results that the table showed before: rows hold objects, and
//!   the free of them can take longer than a frame.
//! - The owner can empty the mailbox: a table that is not painted holds no objects.

use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use fresnel_core::net::task::{Backoff, Restart, Stop, TaskHandle, TaskName, TaskState, Tasks};

/// A view tells that it waits for its worker only after this time. A result for 10,000
/// objects takes some milliseconds, and a note for each new snapshot would flicker.
pub(crate) const SLOW: Duration = Duration::from_millis(250);

/// The identity of a request: the state of the table that it was made for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RequestId {
    /// A number that grows with each change of the sort or of the filter.
    pub(crate) query: u64,
    /// A number that grows with each change of the objects.
    pub(crate) data: u64,
}

/// One result with the identity of its request.
#[derive(Debug)]
pub(crate) struct Done<Out> {
    pub(crate) id: RequestId,
    pub(crate) output: Out,
    /// The time of the computation on the thread.
    pub(crate) time: Duration,
}

/// Tells a computation that a request with a newer query waits, so its result has no use.
pub(crate) struct Cancel<'a> {
    newest_query: &'a AtomicU64,
    query: u64,
}

impl<'a> Cancel<'a> {
    /// The mark for a computation of this query. `newest_query` holds the query of the
    /// newest request.
    pub(crate) fn new(newest_query: &'a AtomicU64, query: u64) -> Self {
        Self { newest_query, query }
    }

    /// A computation reads this between two steps and returns `None` when it is true.
    pub(crate) fn is_set(&self) -> bool {
        self.newest_query.load(Ordering::Relaxed) != self.query
    }
}

/// Wakes the UI for a result. The shell gives a function that asks egui for a repaint.
pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

struct Mailbox<In, Out> {
    /// The newest request with no result.
    wanted: Option<(RequestId, In)>,
    /// The newest result that the table did not take.
    done: Option<Done<Out>>,
    /// The results that the table does not show any more.
    old: Vec<Out>,
    /// False after the owner of the thread dropped. Then the thread ends.
    open: bool,
}

struct Shared<In, Out> {
    mailbox: Mutex<Mailbox<In, Out>>,
    changed: Condvar,
    /// The query of the newest request. A computation reads it with no lock.
    newest_query: AtomicU64,
}

impl<In: Clone, Out> Shared<In, Out> {
    /// Each use holds the lock for one small change. No computation and no free of a
    /// result runs under it, so a panic of a computation cannot poison it with half a change.
    fn mailbox(&self) -> MutexGuard<'_, Mailbox<In, Out>> {
        self.mailbox.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits for a request and gives a copy of it. The request stays in the mailbox: a
    /// run after a panic finds it again. `None` after the owner closed the mailbox.
    fn next(&self) -> Option<(RequestId, In)> {
        let mut mailbox = self.mailbox();
        loop {
            if !mailbox.old.is_empty() {
                let old = std::mem::take(&mut mailbox.old);
                drop(mailbox);
                drop(old);
                mailbox = self.mailbox();
            } else if !mailbox.open {
                return None;
            } else if let Some(wanted) = &mailbox.wanted {
                return Some(wanted.clone());
            } else {
                mailbox = self.changed.wait(mailbox).unwrap_or_else(PoisonError::into_inner);
            }
        }
    }

    /// Puts a result into the mailbox. Its request leaves the mailbox, unless a newer
    /// one took its place. Gives false for a result with no reader: the owner emptied
    /// the mailbox during the computation, and this thread frees the result.
    fn finish(&self, done: Done<Out>) -> bool {
        let mut mailbox = self.mailbox();
        if mailbox.wanted.is_none() {
            drop(mailbox);
            return false;
        }
        let request = mailbox.wanted.take_if(|(id, _)| *id == done.id);
        let replaced = (mailbox.done.replace(done), request);
        drop(mailbox);
        // A result that the table did not take, and the snapshots of the request.
        drop(replaced);
        true
    }

    /// The body of the thread: computes each request to the close of the mailbox.
    fn serve(&self, compute: &impl Fn(&In, &Cancel<'_>) -> Option<Out>, wake: &Wake) {
        while let Some((id, request)) = self.next() {
            let started = Instant::now();
            let output = compute(&request, &Cancel::new(&self.newest_query, id.query));
            drop(request);
            if let Some(output) = output
                && self.finish(Done { id, output, time: started.elapsed() })
            {
                wake();
            }
        }
    }
}

/// Closes the mailbox at its drop, which ends the thread.
struct Door<In, Out>(Arc<Shared<In, Out>>);

impl<In, Out> Drop for Door<In, Out> {
    fn drop(&mut self) {
        self.0.mailbox.lock().unwrap_or_else(PoisonError::into_inner).open = false;
        self.0.changed.notify_all();
    }
}

/// The owner of the worker thread of one table. The thread ends when this value drops.
///
/// The task handle is the first field: it drops before the door, so the end of the
/// thread is the stop of its task and not a fault.
pub(crate) struct Worker<In, Out> {
    task: TaskHandle,
    tasks: Tasks,
    door: Door<In, Out>,
}

impl<In: Clone + Send + 'static, Out: Send + 'static> Worker<In, Out> {
    /// Starts the thread. `compute` gives `None` only after its `Cancel` is set.
    pub(crate) fn start(
        tasks: &Tasks,
        name: TaskName,
        wake: Wake,
        compute: impl Fn(&In, &Cancel<'_>) -> Option<Out> + Send + 'static,
    ) -> Self {
        let mailbox = Mailbox { wanted: None, done: None, old: Vec::new(), open: true };
        let shared =
            Arc::new(Shared { mailbox: Mutex::new(mailbox), changed: Condvar::new(), newest_query: AtomicU64::new(0) });
        let body = {
            let shared = Arc::clone(&shared);
            move |_stop: &Stop| {
                shared.serve(&compute, &wake);
                Ok::<(), Infallible>(())
            }
        };
        let task = tasks.start_thread(name, Restart::Always(Backoff::default()), body);
        Self { task, tasks: tasks.clone(), door: Door(shared) }
    }

    /// Asks for the rows of a request. It takes the place of a request that waits. The
    /// computation in progress goes on, and it stops only if its query is not this one.
    pub(crate) fn submit(&self, id: RequestId, request: In) {
        let shared = &self.door.0;
        let replaced = {
            let mut mailbox = shared.mailbox();
            shared.newest_query.store(id.query, Ordering::Relaxed);
            mailbox.wanted.replace((id, request))
        };
        drop(replaced);
        shared.changed.notify_one();
    }

    /// The newest result, if one arrived after the last call.
    pub(crate) fn take(&self) -> Option<Done<Out>> {
        self.door.0.mailbox().done.take()
    }

    /// Empties the mailbox, for a table that is not painted: a request and a result hold
    /// objects. The thread frees the result. A computation in progress goes on to its end
    /// and finds no request in the mailbox then, so its result has no reader.
    pub(crate) fn clear(&self) {
        let shared = &self.door.0;
        let request = {
            let mut mailbox = shared.mailbox();
            if let Some(done) = mailbox.done.take() {
                mailbox.old.push(done.output);
            }
            mailbox.wanted.take()
        };
        drop(request);
        shared.changed.notify_one();
    }

    /// Gives a result that the table does not show any more to the thread, which frees it.
    pub(crate) fn discard(&self, output: Out) {
        let shared = &self.door.0;
        shared.mailbox().old.push(output);
        shared.changed.notify_one();
    }

    /// Why the thread does not compute now: its last run ended, for example with a
    /// panic. `None` while it runs. The journal has the same text.
    pub(crate) fn failure(&self) -> Option<String> {
        let status = self.tasks.snapshot().into_iter().find(|status| status.id == self.task.id())?;
        match status.state {
            TaskState::Running => None,
            TaskState::Backoff { last, .. } => Some(format!("{last}; it starts again")),
            TaskState::Ended(end) => Some(end.to_string()),
        }
    }
}

#[cfg(test)]
mod tests;
