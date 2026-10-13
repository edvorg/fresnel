//! The watch loop: the task that keeps one store equal to the objects of its cluster
//! (plan section 9.1).
//!
//! The loop reads a list in pages and then follows a watch from the resource version of
//! the list. After each end of a watch it starts a new one from the last version that it
//! saw. Only HTTP 410 makes a new list necessary. The store shows each step as its state,
//! and it stays live through the renewal of a watch that the server ended at its timeout.
//!
//! - `list`: the list in pages. `stream`: one watch. `recover`: what follows a failure.
//! - `source`: the two requests. `lanes`: the connections of the cluster. `gate`: the
//!   limit for lists.

mod gate;
mod lanes;
mod list;
mod recover;
mod source;
mod stream;

use std::ops::RangeInclusive;
use std::panic::{AssertUnwindSafe, resume_unwind};
use std::sync::Arc;
use std::time::Duration;

use fresnel_net::task::{Backoff, panic_message};
use fresnel_net::{Listener, Signal, WallClock};
use futures_util::FutureExt as _;

pub(crate) use gate::ListGate;
#[cfg(test)]
pub(crate) use gate::Turn;
pub(crate) use lanes::{LaneStates, Lanes, follow};
pub(crate) use source::{GatewaySource, Source};

use crate::error::CoreError;
use crate::kinds::{Kind, Object};
use crate::published::Publisher;
use crate::store::{Snapshot, StoreState, Wait, Writer};
use lanes::{Lane, Readiness};
use recover::Attempts;

/// The numbers of the watch loop. Each one is a first value (plan section 16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchSettings {
    /// The most objects of one list page.
    pub page_size: u32,
    /// The lists that one cluster runs at the same time.
    pub lists: usize,
    /// The server ends a watch after a random time in this range, in seconds. A short
    /// time gives a quiet kind a sign of life soon. The random part keeps the watches of
    /// one cluster from one common end.
    pub server_timeout: RangeInclusive<u32>,
    /// A watch with no event and no bookmark for its server timeout and this time is
    /// stalled. The loop starts a new one.
    pub stall_margin: Duration,
    /// The shortest time between two snapshots with new objects.
    pub batch: Duration,
    /// The pauses after a request that failed. A watch that was open for `reset_after`
    /// or longer, or that gave an event, ends the sequence of failures.
    pub backoff: Backoff,
    /// The time before a new list of a kind that the user cannot read.
    pub forbidden_retry: Duration,
    /// The time before a new list of a kind that the cluster does not have.
    pub unsupported_retry: Duration,
}

impl Default for WatchSettings {
    fn default() -> Self {
        Self {
            page_size: 500,
            lists: 4,
            server_timeout: 60..=90,
            stall_margin: Duration::from_secs(10),
            batch: Duration::from_millis(30),
            // The longest pause is far below the recovery limit of 10 s.
            backoff: Backoff {
                first: Duration::from_millis(100),
                max: Duration::from_secs(3),
                jitter_percent: 50,
                reset_after: Duration::from_secs(1),
            },
            forbidden_retry: Duration::from_secs(60),
            unsupported_retry: Duration::from_secs(600),
        }
    }
}

impl WatchSettings {
    /// The problem of settings that no loop can run with. The server takes a watch
    /// timeout below 295 s only.
    pub(crate) fn problem(&self) -> Option<CoreError> {
        let (start, end) = (*self.server_timeout.start(), *self.server_timeout.end());
        let problem = if start == 0 || start > end || end >= 295 {
            "the server timeout of a watch is a range from 1 s to 294 s"
        } else if self.page_size == 0 {
            "a list page has one object at least"
        } else {
            return None;
        };
        Some(CoreError::invalid(problem))
    }
}

/// What the loop of one store works with. Each run of the task gets the same parts.
pub(crate) struct Parts<T, S> {
    pub(crate) kind: Kind,
    pub(crate) source: S,
    pub(crate) publisher: Publisher<Snapshot<T>>,
    pub(crate) lanes: Lanes,
    /// The one signal of the cluster: a wake, a network change and "retry now".
    pub(crate) signal: Signal,
    pub(crate) gate: Arc<ListGate>,
    pub(crate) settings: Arc<WatchSettings>,
    pub(crate) clock: WallClock,
    /// The most objects of the store. `None`: no limit.
    pub(crate) max_items: Option<usize>,
}

/// One run of the loop.
struct Loop<'a, T, S> {
    parts: &'a Parts<T, S>,
    writer: Writer<T>,
    lanes: Lanes,
    retry_now: Listener,
    /// The resource version that the next watch starts after. `None`: a list comes first.
    cursor: Option<String>,
    /// True after the first list of this run got its slot. Each later list has no
    /// priority over the lists of the other stores.
    listed_before: bool,
    /// True after a watch that worked and that the server ended at its timeout. The next
    /// watch is then a renewal: the store stays live while it starts.
    renewal: bool,
    attempts: Attempts,
}

impl<T: Object, S: Source<T>> Parts<T, S> {
    /// Shows a fault of the task as the state of the store. The objects stay.
    fn show_fault(&self, reason: &str) {
        let last = self.publisher.read();
        self.publisher.publish(last.in_state(StoreState::InternalError { reason: reason.into() }, self.clock.now()));
    }
}

/// One run of the task of a store. A run that ends with an error or with a panic leaves
/// the state "internal error" (design rule 1). The start function then gives the report
/// for the journal and starts the task again: the panic goes on to it.
pub(crate) async fn run<T: Object, S: Source<T>>(parts: Arc<Parts<T, S>>) -> Result<(), CoreError> {
    match AssertUnwindSafe(keep_current(&parts)).catch_unwind().await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            parts.show_fault(&error.to_string());
            Err(error)
        }
        Err(panic) => {
            parts.show_fault(&format!("the task of the store panicked: {}", panic_message(panic.as_ref())));
            resume_unwind(panic)
        }
    }
}

/// Keeps the store current until the task stops. It returns only with a fault of the
/// loop itself. Each run starts with a list: a run that ended can have lost an event.
async fn keep_current<T: Object, S: Source<T>>(parts: &Parts<T, S>) -> Result<(), CoreError> {
    let (publisher, clock) = (parts.publisher.clone(), parts.clock.clone());
    let writer = Writer::resume(publisher, clock, parts.settings.batch, parts.max_items);
    let mut steps = Loop {
        parts,
        writer,
        lanes: parts.lanes.clone(),
        retry_now: parts.signal.listen(),
        cursor: None,
        listed_before: false,
        renewal: false,
        attempts: Attempts::default(),
    };
    loop {
        let step = match steps.cursor.clone() {
            None => steps.list().await,
            Some(from) => steps.watch(&from).await,
        };
        if let Err(failure) = step {
            steps.recover(failure).await?;
        }
    }
}

impl<T: Object, S: Source<T>> Loop<'_, T, S> {
    /// Waits until a request of the lane can go out, and shows what the store waits for.
    /// A request into a connection that is gone would only fail at its time limit.
    async fn ready(&mut self, lane: Lane) -> Result<(), CoreError> {
        loop {
            match self.lanes.readiness(lane, self.parts.clock.now()) {
                Readiness::Ready => return Ok(()),
                Readiness::Wait(wait) => {
                    self.enter(StoreState::Waiting(wait));
                    self.lanes.changed().await?;
                }
                Readiness::Throttled(until) => self.wait_for_server(until).await,
            }
        }
    }

    /// Waits for the answer to a request. The connection layer holds a request while the
    /// server throttles the cluster (HTTP 429), and the store shows that state then.
    async fn answer<R>(&mut self, request: impl Future<Output = Result<R, CoreError>>) -> Result<R, CoreError> {
        tokio::pin!(request);
        let mut shown = None;
        loop {
            tokio::select! {
                answer = &mut request => return answer,
                until = self.lanes.throttled(self.parts.clock.now(), shown) => {
                    self.enter(StoreState::Throttled { until });
                    shown = Some(until);
                }
            }
        }
    }

    /// Shows a step of the loop. The state of a refusal stays until a request gets an
    /// answer: the store asks again from time to time, and the UI must not show each try.
    fn enter(&mut self, state: StoreState) {
        let refused = matches!(self.writer.state(), StoreState::Forbidden { .. } | StoreState::NotSupported { .. });
        if !refused {
            self.writer.set_state(state);
        }
    }

    /// Shows that a request starts: an attempt after failures, or the step itself.
    fn start_request(&mut self, step: StoreState) {
        let attempt = self.attempts.failed().map(|(failures, reason)| (failures, reason.clone()));
        self.enter(attempt.map_or(step, |(failures, reason)| StoreState::Attempt { failures, reason }));
    }
}

#[cfg(test)]
mod tests;
