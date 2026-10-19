//! What follows a request of the loop that failed: a state of the store and a wait.

use std::time::{Duration, SystemTime};

use kube::core::Status;
use tokio::time::sleep;

use super::source::Source;
use super::{Loop, StoreState};
use crate::error::CoreError;
use crate::kinds::Object;
use crate::net::ErrorClass;

/// Why a step of the loop ended early.
#[derive(Debug)]
pub(super) enum Failure {
    /// The connection of the watch is gone. This is a state of the channel and not a
    /// failure of a request: the loop waits for the channel and counts nothing.
    ChannelLost,
    /// The server no longer has the resource version of the request (HTTP 410). Only a
    /// new list gives the current objects.
    Gone(CoreError),
    /// A watch that worked ended with an error. The loop starts the next watch at once: a
    /// second failure with no answer between gets the first pause.
    Interrupted(CoreError),
    /// Each other failure.
    Failed(CoreError),
    /// A fault of the loop itself. The run ends, and the task starts again.
    Fault(CoreError),
}

impl From<CoreError> for Failure {
    /// The error of a request, or of an answer that the store cannot use.
    fn from(error: CoreError) -> Self {
        if error.status() == Some(GONE) { Self::Gone(error) } else { Self::Failed(error) }
    }
}

const GONE: u16 = 410;

impl Failure {
    /// The failure for an `ERROR` event of a watch. The text keeps the reason and the
    /// code of the server.
    pub(super) fn of_event(status: &Status) -> Self {
        let (code, reason) = (status.code, &status.reason);
        let message = if status.message.is_empty() { "the watch ended with an error" } else { &status.message };
        let text = if reason.is_empty() {
            format!("{message} (HTTP {code})")
        } else {
            format!("{message} ({reason}, HTTP {code})")
        };
        Self::from(CoreError::answered(code, text))
    }
}

/// The failures in sequence of one loop.
#[derive(Debug, Default)]
pub(super) struct Attempts {
    /// The requests that failed since the last complete list or watch that worked.
    failures: u32,
    /// Why the last one failed.
    last: Option<CoreError>,
    /// The HTTP 410 answers since the last watch that worked.
    gone: u32,
}

impl Attempts {
    /// The count of failures in sequence with the reason of the last one. `None` with no
    /// failure.
    pub(super) fn failed(&self) -> Option<(u32, &CoreError)> {
        Some(self.failures).zip(self.last.as_ref())
    }

    fn count(&mut self, reason: CoreError) {
        self.failures = self.failures.saturating_add(1);
        self.last = Some(reason);
    }

    /// A list is complete: the sequence of failures ends (plan section 9.1). After 7
    /// resets in one minute, the loop of the prototype waited 17 s, because it counted
    /// each one.
    ///
    /// One good page does not end the sequence. A list that fails at a later page each
    /// time would start again after the first pause for all time, with all its pages.
    pub(super) fn listed(&mut self) {
        self.failures = 0;
        self.last = None;
    }

    /// A watch gave data or stayed open. A good list does not end a sequence of HTTP 410:
    /// a server that gives a list and then HTTP 410 to each watch must not get a new list
    /// with no pause.
    pub(super) fn watch_worked(&mut self) {
        *self = Self::default();
    }
}

/// What the loop does after an error of a request.
enum Remedy {
    Forbidden,
    NotSupported,
    Throttled,
    Backoff,
}

/// `throttled` is true while the connection layer holds each request for the server.
fn remedy(error: &CoreError, throttled: bool) -> Remedy {
    match (error.status(), error.class()) {
        (Some(403), _) => Remedy::Forbidden,
        (Some(404), _) => Remedy::NotSupported,
        (Some(429), _) => Remedy::Throttled,
        // The connection layer did not send the request: its time limit ends before the
        // throttle of the server. That error has no status.
        (None, ErrorClass::Server) if throttled => Remedy::Throttled,
        _ => Remedy::Backoff,
    }
}

impl<T: Object, S: Source<T>> Loop<'_, T, S> {
    /// Shows the failure as the state of the store and waits before the next step. The
    /// objects of the store stay (design rule 5).
    pub(super) async fn recover(&mut self, failure: Failure) -> Result<(), CoreError> {
        self.writer.cancel_list();
        let now = self.parts.clock.now();
        let throttled_until = self.lanes.throttled_until().filter(|until| *until > now);
        match failure {
            Failure::Fault(error) => return Err(error),
            Failure::ChannelLost => self.lost = true,
            Failure::Interrupted(error) => {
                // A bad line of the server tells of no lost connection.
                self.lost |= error.class() != ErrorClass::Invalid;
                self.attempts.count(error);
            }
            Failure::Gone(error) => {
                self.cursor = None;
                self.attempts.gone = self.attempts.gone.saturating_add(1);
                // The first HTTP 410 gets a new list at once. Each one after it gets a pause.
                if self.attempts.gone > 1 {
                    self.pause(self.attempts.gone - 1, error).await;
                }
            }
            Failure::Failed(error) => {
                // A request with no answer for the network or for its time limit can have
                // lost its connection, and the next connection can reach an other cluster.
                self.lost |= matches!(error.class(), ErrorClass::Network | ErrorClass::Timeout);
                match remedy(&error, throttled_until.is_some()) {
                    // A refusal can end, so the store asks again, with a list: after a watch
                    // that the server refused, the objects of the store are old.
                    Remedy::Forbidden => {
                        self.cursor = None;
                        self.writer.set_state(StoreState::Forbidden { reason: error });
                        self.rest(self.parts.settings.forbidden_retry).await;
                    }
                    Remedy::NotSupported => {
                        self.cursor = None;
                        self.writer.set_state(StoreState::NotSupported { reason: error });
                        self.rest(self.parts.settings.unsupported_retry).await;
                    }
                    Remedy::Throttled => self.wait_for_server(throttled_until.unwrap_or(now + NO_RETRY_AFTER)).await,
                    Remedy::Backoff => {
                        self.attempts.count(error.clone());
                        self.pause(self.attempts.failures, error).await;
                    }
                }
            }
        }
        Ok(())
    }

    /// Waits before the next request. The pause grows with the count of failures in
    /// sequence, to a limit.
    async fn pause(&mut self, failures: u32, reason: CoreError) {
        let pause = self.parts.settings.backoff.pause(failures, fastrand::u32(..));
        let until = self.parts.clock.now() + pause;
        self.writer.set_state(StoreState::Backoff { failures, reason, until });
        self.rest(pause).await;
    }

    /// Waits for `pause`, or less: the one signal of the cluster ends each retry delay, and
    /// so does a connection that returns. That one was lost: the next watch checks the cluster.
    async fn rest(&mut self, pause: Duration) {
        tokio::select! {
            _ = self.retry_now.sleep(pause) => {}
            () = self.lanes.returned() => self.lost = true,
        }
    }

    /// Waits until the server takes requests again (HTTP 429). The signal of the cluster
    /// does not end this wait: the app obeys the time of the server.
    pub(super) async fn wait_for_server(&mut self, until: SystemTime) {
        self.enter(StoreState::Throttled { until });
        let left = until.duration_since(self.parts.clock.now()).unwrap_or_default();
        sleep(left).await;
    }
}

/// The wait after a throttle answer when the connection layer gives no time.
const NO_RETRY_AFTER: Duration = Duration::from_secs(1);
