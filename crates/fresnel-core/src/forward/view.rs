//! What the view of a forward shows (plan section 12.3): the target pod, the local address,
//! the state, the last error, the time of the next attempt, the connections, the bytes and
//! the age of the data of the target.

use std::net::SocketAddr;
use std::time::SystemTime;

use super::resolve::TargetPod;
use super::rule::ForwardRule;
use super::state::ForwardState;
use crate::kinds::Kind;

/// How fresh the data is that chose the target pod: the stores of the pods, and of the
/// Service or the workload of the rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetData {
    /// No store had a complete list yet.
    NotLoaded,
    /// Each store is live: each change of the cluster arrives.
    Current,
    /// The store of this kind is not live since this time: the pod can be gone.
    Old { kind: Kind, since: SystemTime },
}

/// The connections of the local clients, from the start of the forward.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Connections {
    /// Open now, also those that wait for a target.
    pub open: u64,
    /// Accepted in total.
    pub total: u64,
    /// Ended with a reset: no target in the wait time, or a tunnel that broke.
    pub reset: u64,
}

/// The bytes of the tunnels, from the start of the forward.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bytes {
    /// From the local clients to the pods.
    pub sent: u64,
    /// From the pods to the local clients.
    pub received: u64,
}

/// One forward at one moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardView {
    pub rule: ForwardRule,
    pub state: ForwardState,
    /// The wall clock time of the last step of the state.
    pub since: SystemTime,
    /// The local addresses, each with its port. Empty with no listener.
    pub addresses: Vec<SocketAddr>,
    /// The pod of the next tunnels. `None` with no pod.
    pub target: Option<TargetPod>,
    pub data: TargetData,
    /// The last failure of a tunnel or of the forward, with its wall clock time.
    pub last_error: Option<(SystemTime, String)>,
    pub connections: Connections,
    pub bytes: Bytes,
}

impl ForwardView {
    /// A forward that did not start yet.
    pub fn new(rule: ForwardRule, state: ForwardState, now: SystemTime) -> Self {
        Self {
            rule,
            state,
            since: now,
            addresses: Vec::new(),
            target: None,
            data: TargetData::NotLoaded,
            last_error: None,
            connections: Connections::default(),
            bytes: Bytes::default(),
        }
    }

    /// The address that a user copies or opens: the first one, as `127.0.0.1:8080`. `None`
    /// with no listener.
    pub fn address(&self) -> Option<SocketAddr> {
        self.addresses.first().copied()
    }
}
