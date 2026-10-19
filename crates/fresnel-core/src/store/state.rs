//! The state of one store (plan section 9.7). The UI shows it with the data of the store.

use std::fmt::{self, Display};
use std::sync::Arc;
use std::time::SystemTime;

use crate::error::CoreError;

/// What a store waits for before its next request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Wait {
    /// A connection of the cluster that answers.
    Channel,
    /// A token of the credential plugin.
    Credential,
    /// One of the lists that a cluster runs at the same time.
    ListSlot,
    /// The start of its watch after a list or after a fault: a free stream of the
    /// connection and the answer of the server.
    Stream,
    /// The check after a lost connection that the cluster behind the new one is still the
    /// cluster of its objects: its identity and its version.
    Cluster,
}

/// The state of one store. In each state, the store shows the objects that it has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreState {
    /// The store sends no request now. It waits for something that some other part gives.
    Waiting(Wait),
    /// A list is in progress, with the count of objects that arrived.
    Listing {
        /// The objects of the pages that arrived.
        loaded: usize,
    },
    /// The watch is open: each change of the cluster arrives. The server ends each watch
    /// after its timeout, and the state stays while the next one starts: that start has
    /// a time limit, and the changes of that time arrive with the new watch.
    Live,
    /// A request is in progress after failures. No answer arrived yet.
    Attempt {
        /// The count of failures in sequence before this request.
        failures: u32,
        /// Why the last request failed.
        reason: CoreError,
    },
    /// The last request failed. The next one starts at `until`. A wake, a network change,
    /// "retry now" and a connection that returns end the pause.
    Backoff {
        /// The count of failures in sequence.
        failures: u32,
        /// Why the last request failed.
        reason: CoreError,
        /// The wall clock time of the next request.
        until: SystemTime,
    },
    /// The server asked the app to wait (HTTP 429 with `Retry-After`).
    Throttled {
        /// The wall clock time of the next request.
        until: SystemTime,
    },
    /// The user cannot read the kind (HTTP 403). The store asks again after some time.
    Forbidden {
        /// The message of the server.
        reason: CoreError,
    },
    /// The cluster does not have the kind (HTTP 404). The store asks again after some time.
    NotSupported {
        /// The message of the server.
        reason: CoreError,
    },
    /// The task of the store failed, for example with a panic. The task starts again.
    InternalError {
        /// What failed.
        reason: Arc<str>,
    },
    /// The app stopped the store. Its objects do not change again.
    Stopped,
}

impl StoreState {
    /// True while each change of the cluster arrives.
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Live)
    }
}

impl Display for StoreState {
    /// The state in words, with no time: the UI gives a time in the form of the user.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Waiting(Wait::Channel) => f.write_str("waits for a connection"),
            Self::Waiting(Wait::Credential) => f.write_str("waits for a credential"),
            Self::Waiting(Wait::ListSlot) => f.write_str("waits for a list slot"),
            Self::Waiting(Wait::Stream) => f.write_str("waits for the start of its watch"),
            Self::Waiting(Wait::Cluster) => f.write_str("checks the cluster"),
            Self::Listing { loaded } => write!(f, "list in progress, {loaded} objects"),
            Self::Live => f.write_str("live"),
            Self::Attempt { failures, reason } => write!(f, "attempt after {failures} failures: {reason}"),
            Self::Backoff { failures, reason, .. } => write!(f, "pause after {failures} failures: {reason}"),
            Self::Throttled { .. } => f.write_str("throttled by the server"),
            Self::Forbidden { reason } => write!(f, "forbidden: {reason}"),
            Self::NotSupported { reason } => write!(f, "not supported: {reason}"),
            Self::InternalError { reason } => write!(f, "internal error: {reason}"),
            Self::Stopped => f.write_str("stopped"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_state_has_its_words_and_only_one_state_is_live() {
        let reason = || CoreError::answered(403, "pods is forbidden (HTTP 403)");
        let until = SystemTime::UNIX_EPOCH;
        let states = [
            (StoreState::Waiting(Wait::Channel), "waits for a connection"),
            (StoreState::Waiting(Wait::Credential), "waits for a credential"),
            (StoreState::Waiting(Wait::ListSlot), "waits for a list slot"),
            (StoreState::Waiting(Wait::Stream), "waits for the start of its watch"),
            (StoreState::Waiting(Wait::Cluster), "checks the cluster"),
            (StoreState::Listing { loaded: 500 }, "list in progress, 500 objects"),
            (StoreState::Live, "live"),
            (
                StoreState::Attempt { failures: 2, reason: reason() },
                "attempt after 2 failures: pods is forbidden (HTTP 403)",
            ),
            (
                StoreState::Backoff { failures: 3, reason: reason(), until },
                "pause after 3 failures: pods is forbidden (HTTP 403)",
            ),
            (StoreState::Throttled { until }, "throttled by the server"),
            (StoreState::Forbidden { reason: reason() }, "forbidden: pods is forbidden (HTTP 403)"),
            (StoreState::NotSupported { reason: reason() }, "not supported: pods is forbidden (HTTP 403)"),
            (StoreState::InternalError { reason: "index 4 of 3".into() }, "internal error: index 4 of 3"),
            (StoreState::Stopped, "stopped"),
        ];
        for (state, words) in &states {
            assert_eq!(state.to_string(), *words);
        }
        let live: Vec<_> = states.iter().filter(|(state, _)| state.is_live()).map(|(_, words)| *words).collect();
        assert_eq!(live, ["live"]);
    }
}
