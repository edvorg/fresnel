//! The state of a forward (plan section 12.3), and the facts that decide it.
//!
//! The state is not stored: each change of a fact computes it again (port forward prototype,
//! `supervisor/model`). The facts: the listener, the path to the cluster, the target of the
//! stores, the tunnels that failed in a row, a connection that got no tunnel and the refusal
//! of the server.
//!
//! The path is the one check of the cluster: its channels, which ping each 10 s. Each forward
//! of a cluster reads it, so six idle forwards show a silent path at the same moment; in the
//! prototype, each forward had its own check, and the sixth one showed it after 39 s.

use std::fmt::{self, Display};
use std::net::SocketAddr;
use std::time::SystemTime;

use fresnel_net::{ChannelState, ClusterStatus};

use super::resolve::Resolution;
use crate::app::Health;

/// The tunnels that fail in a row, with no tunnel between them that carried bytes of the pod,
/// before the state degrades. A connection tries again after a failure in its wait, so one
/// failure is no failed client.
pub(super) const FAILURES: u32 = 3;

/// The state of one forward.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForwardState {
    /// The user stopped it: no listener.
    Stopped,
    /// The listener binds, or the forward waits for the first connection of its cluster or
    /// the first list of its stores: the text tells which.
    Starting(String),
    /// A local connection gets a tunnel to the target pod now.
    Active,
    /// The listener is open, and a local connection gets no tunnel now: no pod can take
    /// one, the last connection that tried got none, or each of the last tunnels failed.
    /// The text tells why.
    Degraded(String),
    /// The path to the cluster is lost, or its first connect failed: its channels connect
    /// again with no end.
    Reconnecting {
        /// The number of the connect attempt of the cluster, when known.
        attempt: Option<u32>,
        /// The wall clock time of the next attempt. `None`: it runs now.
        next_retry: Option<SystemTime>,
        /// Why the path is lost.
        reason: String,
    },
    /// The forward cannot work until the user acts: the local port is in use, or the server
    /// refuses the forward. No retry runs.
    Failed(String),
}

impl ForwardState {
    /// The name of the state, as a column shows it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting(_) => "starting",
            Self::Active => "active",
            Self::Degraded(_) => "degraded",
            Self::Reconnecting { .. } => "reconnecting",
            Self::Failed(_) => "failed",
        }
    }

    /// The same step of the state: a new time of the next attempt is not a new step, so it
    /// gives no journal entry.
    pub(super) fn same_step(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Reconnecting { reason: one, .. }, Self::Reconnecting { reason: two, .. }) => one == two,
            _ => self == other,
        }
    }
}

impl Display for ForwardState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped | Self::Active => f.write_str(self.name()),
            Self::Starting(reason) | Self::Degraded(reason) | Self::Failed(reason) => {
                write!(f, "{}: {reason}", self.name())
            }
            Self::Reconnecting { attempt: Some(attempt), reason, .. } => {
                write!(f, "reconnecting, attempt {attempt}: {reason}")
            }
            Self::Reconnecting { attempt: None, reason, .. } => write!(f, "reconnecting: {reason}"),
        }
    }
}

/// The path to the cluster, from the state of its channels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Path {
    /// The first connect of the cluster runs.
    First,
    /// A request got an answer on a channel of each purpose.
    Up,
    /// The channels lost their connections, or the credential failed.
    Down { attempt: Option<u32>, next_retry: Option<SystemTime>, reason: String },
}

impl Path {
    /// The path of a connected cluster. `reason` is the text of the cluster health. After
    /// the path was up, a channel that connects again shows a lost path: its first attempt
    /// after a silence can wait for the full time limit of a connect.
    pub(super) fn of(status: &ClusterStatus, reason: Option<String>, was_up: bool) -> Self {
        let reason = || reason.clone().unwrap_or_else(|| "the cluster has no connection".to_owned());
        let channel = status.requests.channels.first();
        let (attempt, next_retry) = match channel.map(|channel| (&channel.state, channel.since)) {
            Some((ChannelState::Backoff { failures, pause }, since)) => (Some(failures + 1), since.checked_add(*pause)),
            Some((ChannelState::Connecting { attempt }, _)) => (Some(*attempt), None),
            _ => (None, None),
        };
        match Health::of_status(status) {
            Health::Live => Self::Up,
            Health::Connecting if !was_up => Self::First,
            Health::Connecting | Health::NoConnection | Health::NoCredential | Health::Off | Health::Failed => {
                Self::Down { attempt, next_retry, reason: reason() }
            }
        }
    }
}

/// The listener of a forward.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Listener {
    Binding,
    Bound(Vec<SocketAddr>),
    /// The bind failed, with its reason.
    Failed(String),
}

/// The facts of one forward at one moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Facts {
    pub(super) listener: Listener,
    /// The path to the cluster, or why the forward has no connected cluster.
    pub(super) path: Result<Path, String>,
    /// True after the path was up one time: a cluster that is not connected after it lost
    /// its connections, which degrades the forward, and before it the forward starts.
    pub(super) was_up: bool,
    /// What the stores give. `None` before the first read.
    pub(super) resolution: Option<Resolution>,
    /// The tunnels that failed in a row, and the text of the last failure.
    pub(super) failures: (u32, Option<String>),
    /// The last connection that tried got no tunnel in its wait, for this reason.
    pub(super) no_tunnel: Option<String>,
    /// The server refused the forward, for example with HTTP 403.
    pub(super) refused: Option<String>,
}

impl Facts {
    pub(super) fn new() -> Self {
        Self {
            listener: Listener::Binding,
            path: Err("the cluster connects".to_owned()),
            was_up: false,
            resolution: None,
            failures: (0, None),
            no_tunnel: None,
            refused: None,
        }
    }

    /// The state that the facts give.
    pub(super) fn state(&self) -> ForwardState {
        match &self.listener {
            Listener::Failed(reason) => return ForwardState::Failed(reason.clone()),
            Listener::Binding => return ForwardState::Starting("the local port binds".to_owned()),
            Listener::Bound(_) => {}
        }
        if let Some(reason) = &self.refused {
            return ForwardState::Failed(reason.clone());
        }
        let path = match &self.path {
            Ok(path) => path,
            Err(reason) if self.was_up => return ForwardState::Degraded(reason.clone()),
            Err(reason) => return ForwardState::Starting(reason.clone()),
        };
        match (path, &self.resolution) {
            (Path::Down { attempt, next_retry, reason }, _) => {
                ForwardState::Reconnecting { attempt: *attempt, next_retry: *next_retry, reason: reason.clone() }
            }
            (Path::First, _) => ForwardState::Starting("the cluster connects".to_owned()),
            (Path::Up, None) => ForwardState::Starting("the stores of the cluster load".to_owned()),
            (Path::Up, Some(Resolution::NotKnown(reason))) => ForwardState::Starting(reason.clone()),
            (Path::Up, Some(Resolution::NoPod(reason))) => ForwardState::Degraded(reason.clone()),
            (Path::Up, Some(Resolution::Pod(_))) => {
                self.tunnel_fault().map_or(ForwardState::Active, ForwardState::Degraded)
            }
        }
    }

    /// Why a local connection gets no tunnel to the pod of the stores: the last one that
    /// tried got none, or each of the last tunnels failed. An open ends the first, and the
    /// first bytes of a pod end both.
    fn tunnel_fault(&self) -> Option<String> {
        if let Some(reason) = &self.no_tunnel {
            return Some(format!("the last local connection got no tunnel: {reason}"));
        }
        let (count, last) = (self.failures.0, self.failures.1.as_deref().unwrap_or("no reason"));
        (count >= FAILURES).then(|| format!("each of the last {count} tunnels failed: {last}"))
    }

    /// True when a local connection can open a tunnel now: the path is up and the stores give
    /// a pod. A target whose tunnels failed stays open, so one tunnel that carries bytes of
    /// the pod ends that state.
    pub(super) fn opens(&self) -> bool {
        matches!(self.listener, Listener::Bound(_))
            && self.refused.is_none()
            && self.path == Ok(Path::Up)
            && matches!(self.resolution, Some(Resolution::Pod(_)))
    }
}

#[cfg(test)]
mod tests;
