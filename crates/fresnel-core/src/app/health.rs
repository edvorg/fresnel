//! The one state of a cluster that the status bar and the journal show.

use fresnel_net::credential::CredentialState;
use fresnel_net::{ChannelState, ChannelStatus, ClusterStatus};

use crate::cluster::LinkState;

/// The state of the connections of one cluster as one value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Health {
    /// The cluster is not selected. The app holds no connection to it.
    Off,
    /// The connect did not start, for example because of a bad kubeconfig entry.
    Failed,
    /// The first connect of a channel is in progress, its first answer did not arrive
    /// yet, or it waits for the first token of the plugin.
    Connecting,
    /// A request got an answer on a channel of each purpose. On a cluster with a plugin,
    /// a channel sends no request before the first token, so the cluster has its token.
    Live,
    /// A connect failed, and the channel tries again.
    NoConnection,
    /// The credential plugin gave no token, or the server refused its token.
    NoCredential,
}

impl Health {
    /// The state of a cluster of the registry.
    pub fn of(link: &LinkState) -> Self {
        match link {
            LinkState::Off => Self::Off,
            LinkState::Failed(_) => Self::Failed,
            LinkState::On(status) => Self::of_status(status),
        }
    }

    /// The state of a connected cluster. A failed credential wins: its channels can be
    /// live for the rest of the life of an old token.
    pub fn of_status(status: &ClusterStatus) -> Self {
        if credential_failure(status).is_some() {
            return Self::NoCredential;
        }
        let first = first_channels(status);
        let all_live = first.len() == PURPOSES && first.iter().all(|(_, channel)| channel.state == ChannelState::Live);
        if all_live {
            Self::Live
        } else if first.iter().any(|(_, channel)| has_failures(channel)) {
            Self::NoConnection
        } else {
            Self::Connecting
        }
    }
}

/// Why a connected cluster is not live. The text is safe for a log: it has no output of a
/// plugin.
pub(super) fn reason(status: &ClusterStatus) -> Option<String> {
    if let Some(failure) = credential_failure(status) {
        return Some(failure);
    }
    let lost = first_channels(status).into_iter().find_map(|(purpose, channel)| {
        let error = channel.last_error.as_ref().filter(|_| channel.state != ChannelState::Live)?;
        Some(format!("{purpose}: {error}"))
    });
    lost.or_else(|| status.connect.resolve_error.clone())
}

/// The count of channel purposes: watches, requests and log streams.
const PURPOSES: usize = 3;

/// The first channel of each purpose, which is always present, with the name of its purpose.
fn first_channels(status: &ClusterStatus) -> Vec<(&'static str, &ChannelStatus)> {
    let groups = [("watches", &status.watches), ("requests", &status.requests), ("log streams", &status.logs)];
    groups.into_iter().filter_map(|(purpose, group)| Some((purpose, group.channels.first()?))).collect()
}

fn has_failures(channel: &ChannelStatus) -> bool {
    match channel.state {
        ChannelState::Backoff { .. } => true,
        ChannelState::Connecting { attempt } => attempt > 1,
        ChannelState::WaitsForToken | ChannelState::Verifying | ChannelState::Live | ChannelState::Retired => false,
    }
}

/// The text of a failed credential. A plugin that runs again after a failure counts as
/// failed, so the state does not change two times for each attempt.
fn credential_failure(status: &ClusterStatus) -> Option<String> {
    match &status.credential.as_ref()?.state {
        CredentialState::Failed { failure, .. } => Some(failure.to_string()),
        CredentialState::Running { last_failure: Some(failure), .. } => Some(failure.to_string()),
        CredentialState::None | CredentialState::Running { .. } | CredentialState::Valid { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use fresnel_net::credential::{CredentialFailure, CredentialStatus, RunReason};
    use fresnel_net::{ConnectStatus, GroupStatus, ThrottleStatus, UpgradeStatus};

    use super::*;
    use crate::error::CoreError;

    const LIVE: ChannelState = ChannelState::Live;

    fn channel(state: ChannelState, last_error: Option<&str>) -> ChannelStatus {
        ChannelStatus {
            state,
            since: SystemTime::UNIX_EPOCH,
            streams_open: 0,
            stream_budget: 70,
            server_stream_limit: Some(100),
            ping_time: None,
            last_frame_age: None,
            connect_attempts: 1,
            connections: 1,
            last_error: last_error.map(str::to_owned),
        }
    }

    fn status(channels: [ChannelStatus; 3], credential: Option<CredentialState>) -> ClusterStatus {
        let [watches, requests, logs] = channels.map(|channel| GroupStatus { channels: vec![channel], waiting: 0 });
        let connect = ConnectStatus {
            addresses: Vec::new(),
            resolve_error: None,
            connect_time: None,
            hedge_delay: Duration::from_millis(200),
            connects: 3,
            attempts: 3,
        };
        let credential = credential.map(|state| CredentialStatus { state, plugin_runs: 1, unauthorized: 0, proofs: 0 });
        ClusterStatus {
            name: "ams".to_owned(),
            connect,
            watches,
            requests,
            logs,
            credential,
            upgrade: UpgradeStatus::default(),
            throttle: ThrottleStatus::default(),
        }
    }

    fn with_states(states: [ChannelState; 3]) -> ClusterStatus {
        status(states.map(|state| channel(state, None)), None)
    }

    #[test]
    fn a_cluster_is_live_only_when_a_channel_of_each_purpose_is_live() {
        assert_eq!(Health::of_status(&with_states([LIVE, LIVE, LIVE])), Health::Live);
        let first_connect = [LIVE, ChannelState::Verifying, ChannelState::Connecting { attempt: 1 }];
        assert_eq!(Health::of_status(&with_states(first_connect)), Health::Connecting);
        let pause = ChannelState::Backoff { failures: 1, pause: Duration::from_secs(1) };
        assert_eq!(Health::of_status(&with_states([LIVE, LIVE, pause])), Health::NoConnection);
        let again = ChannelState::Connecting { attempt: 2 };
        assert_eq!(Health::of_status(&with_states([again, LIVE, LIVE])), Health::NoConnection);

        // An extra channel that is closed does not count.
        let mut extra = with_states([LIVE, LIVE, LIVE]);
        extra.watches.channels.push(channel(ChannelState::Retired, Some("idle")));
        assert_eq!((Health::of_status(&extra), reason(&extra)), (Health::Live, None));
        let mut no_channel = with_states([LIVE, LIVE, LIVE]);
        no_channel.logs.channels.clear();
        assert_eq!(Health::of_status(&no_channel), Health::Connecting);
    }

    #[test]
    fn a_failed_credential_wins_and_stays_while_the_plugin_runs_again() {
        let failed = CredentialState::Failed {
            failure: CredentialFailure::Refused,
            failures: 2,
            next_attempt: SystemTime::now(),
        };
        let live = [LIVE, LIVE, LIVE].map(|state| channel(state, None));
        let refused = status(live.clone(), Some(failed));
        assert_eq!(Health::of_status(&refused), Health::NoCredential);
        assert_eq!(
            reason(&refused).as_deref(),
            Some("the server refused a new token of the credential plugin (HTTP 401)")
        );

        let running =
            |reason, last_failure| CredentialState::Running { reason, since: SystemTime::UNIX_EPOCH, last_failure };
        let again = status(live.clone(), Some(running(RunReason::Retry, Some(CredentialFailure::Stopped))));
        assert_eq!(Health::of_status(&again), Health::NoCredential);
        let refresh = status(live, Some(running(RunReason::Expiry, None)));
        assert_eq!((Health::of_status(&refresh), reason(&refresh)), (Health::Live, None), "the old token still works");
    }

    #[test]
    fn a_cluster_whose_channels_wait_for_the_first_token_connects() {
        let waiting = || [(); 3].map(|()| channel(ChannelState::WaitsForToken, None));
        let first_run =
            CredentialState::Running { reason: RunReason::Start, since: SystemTime::UNIX_EPOCH, last_failure: None };
        for state in [CredentialState::None, first_run] {
            let waits = status(waiting(), Some(state));
            assert_eq!((Health::of_status(&waits), reason(&waits)), (Health::Connecting, None));
        }
        // One channel has its answer, and the token arrived a moment before.
        let valid = Some(CredentialState::Valid { until: None });
        let mixed = [LIVE, ChannelState::Verifying, ChannelState::WaitsForToken].map(|state| channel(state, None));
        assert_eq!(Health::of_status(&status(mixed, valid.clone())), Health::Connecting);
        let live = [LIVE, LIVE, LIVE].map(|state| channel(state, None));
        assert_eq!(Health::of_status(&status(live, valid)), Health::Live);
    }

    #[test]
    fn the_reason_is_the_error_of_the_first_channel_that_is_not_live() {
        let pause = ChannelState::Backoff { failures: 3, pause: Duration::from_secs(4) };
        let channels = [
            channel(LIVE, Some("an old error of a live channel")),
            channel(pause, Some("connect: no answer in 10s")),
            channel(ChannelState::Connecting { attempt: 1 }, Some("the connection closed")),
        ];
        let mut lost = status(channels, None);
        assert_eq!(reason(&lost).as_deref(), Some("requests: connect: no answer in 10s"));

        lost.requests.channels[0].last_error = None;
        assert_eq!(reason(&lost).as_deref(), Some("log streams: the connection closed"));
        lost.logs.channels[0].last_error = None;
        lost.connect.resolve_error = Some("no address for ams.example".to_owned());
        assert_eq!(reason(&lost).as_deref(), Some("no address for ams.example"));
    }

    #[test]
    fn a_cluster_with_no_connection_is_off_or_failed() {
        assert_eq!(Health::of(&LinkState::Off), Health::Off);
        assert_eq!(Health::of(&LinkState::Failed(CoreError::invalid("no server"))), Health::Failed);
        assert_eq!(Health::of(&LinkState::On(Box::new(with_states([LIVE, LIVE, LIVE])))), Health::Live);
    }
}
