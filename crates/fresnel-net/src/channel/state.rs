//! The state of one channel, as a copy for the Connections view (plan section 8.14).

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use super::streams::Streams;
use crate::changes::Changed;

/// The state of one channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelState {
    /// A connect is in progress.
    Connecting {
        /// The number of the attempt in the sequence of failures, from 1.
        attempt: u32,
    },
    /// The connection is open. No request got an answer on it yet, so nothing shows that
    /// the server works.
    Verifying,
    /// A request got an answer on this connection.
    Live,
    /// A connect failed, or a connection was lost early. The next connect starts after the
    /// pause. A wake, a network change and "retry now" end the pause.
    Backoff {
        /// The count of failures in sequence.
        failures: u32,
        /// The pause from the state change to the next connect.
        pause: Duration,
    },
    /// An extra connection that had no stream for some time. It is closed.
    Retired,
}

/// A copy of the facts of one channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelStatus {
    /// The state.
    pub state: ChannelState,
    /// The wall clock time of the last state change.
    pub since: SystemTime,
    /// The streams that are open now. A stream is open until its response body drops.
    pub streams_open: usize,
    /// The most streams that the channel opens.
    pub stream_budget: usize,
    /// The stream limit that the server sent. `None` before the server sent it.
    pub server_stream_limit: Option<usize>,
    /// The round trip time of the last light request.
    pub ping_time: Option<Duration>,
    /// The time since the last frame of a response.
    pub last_frame_age: Option<Duration>,
    /// The count of connects that started.
    pub connect_attempts: u64,
    /// The count of connections that opened.
    pub connections: u64,
    /// Why the last connection ended or the last connect failed.
    pub last_error: Option<String>,
}

/// The facts that the task of the channel writes.
#[derive(Clone)]
pub(super) struct Facts {
    pub(super) state: ChannelState,
    pub(super) since: SystemTime,
    pub(super) ping_time: Option<Duration>,
    pub(super) connect_attempts: u64,
    pub(super) connections: u64,
    pub(super) last_error: Option<String>,
}

/// Holds the facts. Each use holds the lock for one small change or one copy, so a reader
/// can never stop the task of the channel.
pub(super) struct StatusBoard {
    facts: Mutex<Facts>,
    changed: Changed,
}

impl StatusBoard {
    pub(super) fn new(changed: Changed) -> Self {
        let facts = Facts {
            state: ChannelState::Connecting { attempt: 1 },
            since: SystemTime::now(),
            ping_time: None,
            connect_attempts: 0,
            connections: 0,
            last_error: None,
        };
        Self { facts: Mutex::new(facts), changed }
    }

    /// Changes the facts and tells the readers of the cluster.
    pub(super) fn write(&self, change: impl FnOnce(&mut Facts)) {
        change(&mut self.facts.lock().unwrap_or_else(PoisonError::into_inner));
        self.changed.notify();
    }

    /// Changes the state and records the time of the change.
    pub(super) fn enter(&self, state: ChannelState) {
        self.write(|facts| {
            facts.state = state;
            facts.since = SystemTime::now();
        });
    }

    /// The count of connections that opened.
    pub(super) fn connections(&self) -> u64 {
        self.facts.lock().unwrap_or_else(PoisonError::into_inner).connections
    }

    pub(super) fn read(&self, streams: &Streams) -> ChannelStatus {
        let facts = self.facts.lock().unwrap_or_else(PoisonError::into_inner).clone();
        ChannelStatus {
            state: facts.state,
            since: facts.since,
            streams_open: streams.open(),
            stream_budget: streams.budget(),
            server_stream_limit: streams.server_limit(),
            ping_time: facts.ping_time,
            last_frame_age: streams.last_frame_age(),
            connect_attempts: facts.connect_attempts,
            connections: facts.connections,
            last_error: facts.last_error,
        }
    }
}
