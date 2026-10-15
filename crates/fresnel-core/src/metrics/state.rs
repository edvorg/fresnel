//! The state of one poll store of the sampler (plan sections 9.7 and 9.11): the PodMetrics
//! or the NodeMetrics of one cluster. The UI shows it at each value of the store.

use std::fmt::{self, Display};
use std::sync::Arc;
use std::time::SystemTime;

use fresnel_net::ErrorClass;

use super::wire::Read;
use crate::error::CoreError;

/// What the last poll of one list gave (research 11 section 4.6). In each state, the
/// history keeps its samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PollState {
    /// No poll has ended yet.
    Waiting,
    /// The last poll got the list.
    Ok,
    /// HTTP 404: the cluster has no `metrics.k8s.io`. On DigitalOcean, the user installs
    /// metrics-server from the Marketplace.
    NotInstalled(CoreError),
    /// HTTP 503: the API server has the API, and its backend does not answer.
    Unavailable(CoreError),
    /// HTTP 403: the user cannot list the metrics.
    Forbidden(CoreError),
    /// No answer in the time limit of a read (10 s).
    Timeout(CoreError),
    /// Each other failure: no connection, no credential, or an answer that is not a list.
    Error(CoreError),
    /// The task of the sampler failed, for example with a panic. It starts again.
    InternalError(Arc<str>),
    /// The app stopped the sampler. Its values do not change again.
    Stopped,
}

impl PollState {
    /// The state after a failed poll. A 404 and a 503 come straight from the gateway: it
    /// sends a read one time, with no retry that would turn a 503 into a timeout (research
    /// 11 section 4.1).
    pub(crate) fn of_error(error: CoreError) -> Self {
        match (error.status(), error.class()) {
            (Some(404), _) => Self::NotInstalled(error),
            (Some(503), _) => Self::Unavailable(error),
            (_, ErrorClass::Forbidden) => Self::Forbidden(error),
            (_, ErrorClass::Timeout) => Self::Timeout(error),
            _ => Self::Error(error),
        }
    }

    /// True after a poll that got its list.
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }
}

impl Display for PollState {
    /// The state in words, with no time: the UI gives a time in the form of the user.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Waiting => f.write_str("waits for its first answer"),
            Self::Ok => f.write_str("ok"),
            Self::NotInstalled(reason) => write!(f, "metrics-server not found: {reason}"),
            Self::Unavailable(reason) => write!(f, "metrics-server not available: {reason}"),
            Self::Forbidden(reason) => write!(f, "forbidden: {reason}"),
            Self::Timeout(reason) => write!(f, "no answer in time: {reason}"),
            Self::Error(reason) => write!(f, "failed: {reason}"),
            Self::InternalError(reason) => write!(f, "internal error: {reason}"),
            Self::Stopped => f.write_str("stopped"),
        }
    }
}

/// One poll store: its state and what its answers gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollStatus {
    /// What the last poll gave.
    pub state: PollState,
    /// The time of the system of the last change of the state.
    pub since: SystemTime,
    /// The time of the system of the last answer with the list. The UI tells from it how
    /// fresh the values are: the times of the samples are on the clock of the cluster.
    pub answered: Option<SystemTime>,
    /// The count of answers with the list. A series with the number of the last one has a
    /// value of now.
    pub answers: u64,
    /// The items of the last answer that the sampler could read.
    pub items: usize,
    /// The items of the last answer that the sampler could not read (design rule 12).
    pub unread: u64,
    /// Why the first of them could not be read.
    pub problem: Option<Arc<str>>,
}

impl PollStatus {
    pub(crate) fn new(now: SystemTime) -> Self {
        Self { state: PollState::Waiting, since: now, answered: None, answers: 0, items: 0, unread: 0, problem: None }
    }

    /// Takes the result of one poll.
    pub(crate) fn take<T>(&mut self, result: Result<&Read<T>, CoreError>, now: SystemTime) {
        match result {
            Ok(read) => {
                self.answer(read.items.len(), now);
                (self.unread, self.problem) = (read.unread, read.problem.clone());
            }
            Err(error) => self.set(PollState::of_error(error), now),
        }
    }

    /// Counts one answer with the list, of `items` items that the sampler could read.
    pub(crate) fn answer(&mut self, items: usize, now: SystemTime) {
        self.answered = Some(now);
        self.answers += 1;
        self.items = items;
        self.set(PollState::Ok, now);
    }

    /// A new state. The time of the change stays for the same state again.
    pub(crate) fn set(&mut self, state: PollState, now: SystemTime) {
        if self.state != state {
            self.state = state;
            self.since = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_failure_gives_its_state_and_its_words() {
        let error = |status, text: &str| CoreError::answered(status, format!("{text} (HTTP {status})"));
        let net = |class, text: &str| CoreError::new(class, text.to_owned());
        let cases = [
            (
                error(404, "the server could not find the requested resource"),
                "metrics-server not found: the server could not find the requested resource (HTTP 404)",
            ),
            (error(503, "service unavailable"), "metrics-server not available: service unavailable (HTTP 503)"),
            (error(403, "pods.metrics.k8s.io is forbidden"), "forbidden: pods.metrics.k8s.io is forbidden (HTTP 403)"),
            (net(ErrorClass::Timeout, "no answer in 10 s"), "no answer in time: no answer in 10 s"),
            (error(504, "gateway timeout"), "no answer in time: gateway timeout (HTTP 504)"),
            (net(ErrorClass::Network, "connection refused"), "failed: connection refused"),
            (net(ErrorClass::Auth, "the plugin failed"), "failed: the plugin failed"),
            (error(500, "internal"), "failed: internal (HTTP 500)"),
        ];
        for (error, words) in cases {
            assert_eq!(PollState::of_error(error).to_string(), words);
        }
        assert_eq!(PollState::Waiting.to_string(), "waits for its first answer");
        assert_eq!(PollState::InternalError("index 3 of 2".into()).to_string(), "internal error: index 3 of 2");
        assert_eq!(
            (PollState::Ok.to_string(), PollState::Stopped.to_string()),
            ("ok".to_owned(), "stopped".to_owned())
        );
    }

    #[test]
    fn the_time_of_a_state_stays_for_the_same_state_and_an_answer_counts() {
        let at = |seconds| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
        let mut status = PollStatus::new(at(0));
        let read = Read { items: vec![1, 2], unread: 1, problem: Some("an item has no name".into()) };
        status.take(Ok(&read), at(15));
        status.take(Ok(&read), at(30));
        assert_eq!(
            (status.since, status.answered, status.answers, status.items, status.unread),
            (at(15), Some(at(30)), 2, 2, 1)
        );
        let gone = || CoreError::answered(404, "not found (HTTP 404)");
        status.take(Err::<&Read<u8>, _>(gone()), at(45));
        status.take(Err::<&Read<u8>, _>(gone()), at(60));
        assert_eq!(
            (&status.state, status.since, status.answered),
            (&PollState::NotInstalled(gone()), at(45), Some(at(30)))
        );
    }
}
