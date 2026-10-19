//! What a watch loop needs to know of the connections of its cluster.
//!
//! One task for each cluster follows the state of the connection layer and publishes this
//! small copy. Each store of the cluster reads it. Thus a store sends no request into a
//! connection that is gone, and it starts again at the moment the connection is back: no
//! store waits for the end of a retry delay then (plan section 8.13).

use std::time::SystemTime;

use fresnel_net::credential::CredentialState;
use fresnel_net::{ChannelState, ClusterStatus, GroupStatus};
use tokio::sync::watch;

use crate::error::CoreError;
use crate::store::Wait;

/// The connections that carry a request of the loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    /// The channels for lists.
    Requests,
    /// The channels for watches.
    Watches,
}

/// The state of the channels of one lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Path {
    /// A request got an answer on a channel.
    Live,
    /// No channel has a connection that answers.
    NoChannel,
    /// A channel is open and waits for the first token of the credential plugin.
    NoToken,
}

/// The copy that the task of the cluster publishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LaneStates {
    pub(crate) requests: Path,
    pub(crate) watches: Path,
    /// True when the plugin gave no token or the server refused its token. A request
    /// would then wait for the plugin and fail.
    pub(crate) credential_failed: bool,
    /// The server takes no request until this time (HTTP 429). A time in the past is over.
    pub(crate) throttled_until: Option<SystemTime>,
    /// The count of connections that the cluster opened. Each new one can reach an other
    /// cluster at the same address.
    pub(crate) connects: u64,
    /// What the task of the identity read of the cluster behind the connections.
    pub(crate) identity: Identity,
}

/// The cluster behind the connections, as the task of the identity publishes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Identity {
    /// The number of the cluster: one more each time it is an other cluster (`identity`).
    /// Each store lists again then.
    pub(crate) number: u32,
    /// The count of connections before the start of the last read that told the identity.
    /// Each connection up to that count reaches the cluster with this number.
    pub(crate) checked: u64,
}

/// What a request of one lane can do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Readiness {
    Ready,
    Wait(Wait),
    Throttled(SystemTime),
}

impl LaneStates {
    /// The state before the first copy of the connection layer.
    pub(crate) const UNKNOWN: Self = Self {
        requests: Path::NoChannel,
        watches: Path::NoChannel,
        credential_failed: false,
        throttled_until: None,
        connects: 0,
        identity: Identity { number: 0, checked: 0 },
    };

    /// The states of the connections of the cluster, with the identity that its task read.
    pub(crate) fn of(status: &ClusterStatus, identity: Identity) -> Self {
        Self {
            requests: path(&status.requests),
            watches: path(&status.watches),
            credential_failed: credential_failed(status),
            throttled_until: status.throttle.until,
            connects: status.connect.connects,
            identity,
        }
    }

    /// True when a request of the lane can go out: its channel is live and the credential works.
    fn is_open(&self, lane: Lane) -> bool {
        self.path(lane) == Path::Live && !self.credential_failed
    }

    fn path(&self, lane: Lane) -> Path {
        match lane {
            Lane::Requests => self.requests,
            Lane::Watches => self.watches,
        }
    }

    fn readiness(&self, lane: Lane, now: SystemTime) -> Readiness {
        match (self.path(lane), self.throttled_until.filter(|until| *until > now)) {
            (Path::NoChannel, _) => Readiness::Wait(Wait::Channel),
            (Path::NoToken, _) => Readiness::Wait(Wait::Credential),
            (Path::Live, _) if self.credential_failed => Readiness::Wait(Wait::Credential),
            (Path::Live, Some(until)) => Readiness::Throttled(until),
            (Path::Live, None) => Readiness::Ready,
        }
    }
}

/// One live channel carries the requests of the lane. An extra channel that is closed
/// does not count.
fn path(group: &GroupStatus) -> Path {
    let has = |state: &ChannelState| group.channels.iter().any(|channel| channel.state == *state);
    if has(&ChannelState::Live) {
        Path::Live
    } else if has(&ChannelState::WaitsForToken) {
        Path::NoToken
    } else {
        Path::NoChannel
    }
}

/// A plugin that runs again after a failure counts as failed. A plugin that runs for a new
/// token while the old one is valid does not.
fn credential_failed(status: &ClusterStatus) -> bool {
    match status.credential.as_ref().map(|credential| &credential.state) {
        Some(CredentialState::Failed { .. } | CredentialState::Running { last_failure: Some(_), .. }) => true,
        Some(CredentialState::None | CredentialState::Running { .. } | CredentialState::Valid { .. }) | None => false,
    }
}

/// The body of the task of the cluster: publishes a copy after each change of a state. The
/// identity is the one of the task of the identity.
pub(crate) async fn follow(net: fresnel_net::Cluster, states: watch::Sender<LaneStates>) -> Result<(), CoreError> {
    let mut changes = net.changes();
    loop {
        let status = net.status();
        states.send_if_modified(|states| {
            let current = LaneStates::of(&status, states.identity);
            std::mem::replace(states, current) != current
        });
        changes.next().await;
    }
}

/// The read side of one store.
#[derive(Clone, Debug)]
pub(crate) struct Lanes {
    states: watch::Receiver<LaneStates>,
}

impl Lanes {
    pub(crate) fn new(states: watch::Receiver<LaneStates>) -> Self {
        Self { states }
    }

    pub(crate) fn readiness(&self, lane: Lane, now: SystemTime) -> Readiness {
        self.states.borrow().readiness(lane, now)
    }

    /// The time until which the server takes no request, as the connection layer gave it.
    pub(crate) fn throttled_until(&self) -> Option<SystemTime> {
        self.states.borrow().throttled_until
    }

    /// Waits until the server throttles the cluster to a time after `now` that is not
    /// `shown`, and gives that time.
    pub(crate) async fn throttled(&mut self, now: SystemTime, shown: Option<SystemTime>) -> SystemTime {
        let new = |until: &SystemTime| *until > now && Some(*until) != shown;
        let found = self.states.wait_for(|states| states.throttled_until.as_ref().is_some_and(new)).await;
        match found.map(|states| states.throttled_until) {
            Ok(Some(until)) => until,
            Ok(None) | Err(_) => std::future::pending().await,
        }
    }

    /// Waits for a new copy. The error tells that the cluster is gone.
    pub(crate) async fn changed(&mut self) -> Result<(), CoreError> {
        self.states.changed().await.map_err(|_| gone())
    }

    /// Waits until no channel of the lane is live. A watch on a connection that is gone
    /// gets no event, and its stream can stay open for some time.
    pub(crate) async fn lost(&mut self, lane: Lane) {
        let lost = self.states.wait_for(|states| states.path(lane) != Path::Live).await.map(drop);
        if lost.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// The number of the cluster behind the connections now.
    pub(crate) fn cluster(&self) -> u32 {
        self.states.borrow().identity.number
    }

    /// Waits until the cluster behind the connections is not the one with this number.
    pub(crate) async fn other_cluster(&mut self, number: u32) {
        let other = self.states.wait_for(|states| states.identity.number != number).await.map(drop);
        if other.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// Waits until a request of the lane can go out and, unless `due`, until a connection
    /// opened that the task of the identity did not read the cluster behind. Gives the count
    /// of connections then.
    pub(crate) async fn unread(&mut self, lane: Lane, due: bool) -> Result<u64, CoreError> {
        let unread = |states: &LaneStates| due || states.connects > states.identity.checked;
        let found = self.states.wait_for(|states| states.is_open(lane) && unread(states)).await;
        found.map(|states| states.connects).map_err(|_| gone())
    }

    /// Waits until the count of connections is more than `connects`.
    pub(crate) async fn connected_after(&mut self, connects: u64) {
        let new = self.states.wait_for(|states| states.connects > connects).await.map(drop);
        if new.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// Waits until the two lanes are live after one of them was not. This is the moment a
    /// connection returns. With lanes that stay live, the wait has no end.
    pub(crate) async fn returned(&mut self) {
        let both_live = |states: &LaneStates| states.requests == Path::Live && states.watches == Path::Live;
        let mut was_down = false;
        let returned = self.states.wait_for(|states| {
            let live = both_live(states);
            was_down |= !live;
            was_down && live
        });
        if returned.await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

fn gone() -> CoreError {
    CoreError::invalid("the task that follows the connections of the cluster is gone")
}

#[cfg(test)]
mod tests;
