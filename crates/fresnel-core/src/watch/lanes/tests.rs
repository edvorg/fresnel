use std::time::{Duration, SystemTime};

use fresnel_net::credential::{CredentialFailure, CredentialStatus, RunReason};
use fresnel_net::{ChannelStatus, ConnectStatus, ThrottleStatus, UpgradeStatus};
use futures_util::FutureExt as _;

use super::*;

const EPOCH: SystemTime = SystemTime::UNIX_EPOCH;
const SECOND: Duration = Duration::from_secs(1);
const LIVE: LaneStates =
    LaneStates { requests: Path::Live, watches: Path::Live, credential_failed: false, throttled_until: None };

fn group(states: &[ChannelState]) -> GroupStatus {
    let channel = |state: &ChannelState| ChannelStatus {
        state: state.clone(),
        since: EPOCH,
        streams_open: 0,
        stream_budget: 70,
        server_stream_limit: Some(100),
        ping_time: None,
        last_frame_age: None,
        connect_attempts: 1,
        connections: 1,
        last_error: None,
    };
    GroupStatus { channels: states.iter().map(channel).collect(), waiting: 0 }
}

fn status(watches: &[ChannelState], requests: &[ChannelState], credential: Option<CredentialState>) -> ClusterStatus {
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
        outage: None,
        watches: group(watches),
        requests: group(requests),
        logs: group(&[ChannelState::Live]),
        credential,
        upgrade: UpgradeStatus::default(),
        throttle: ThrottleStatus::default(),
    }
}

#[test]
fn a_lane_is_live_with_one_live_channel_and_tells_a_missing_token_from_a_missing_connection() {
    let pause = ChannelState::Backoff { failures: 2, pause: SECOND };
    let two = [pause.clone(), ChannelState::Live];
    let lanes = LaneStates::of(&status(&two, &[ChannelState::Verifying], None));
    assert_eq!((lanes.watches, lanes.requests), (Path::Live, Path::NoChannel), "an answer makes a channel live");
    assert_eq!(lanes.readiness(Lane::Watches, EPOCH), Readiness::Ready);
    assert_eq!(lanes.readiness(Lane::Requests, EPOCH), Readiness::Wait(Wait::Channel));

    let waits = [ChannelState::WaitsForToken];
    let first_run = CredentialState::Running { reason: RunReason::Start, since: EPOCH, last_failure: None };
    let lanes = LaneStates::of(&status(&waits, &[pause, ChannelState::Retired], Some(first_run)));
    assert_eq!((lanes.watches, lanes.requests, lanes.credential_failed), (Path::NoToken, Path::NoChannel, false));
    assert_eq!(lanes.readiness(Lane::Watches, EPOCH), Readiness::Wait(Wait::Credential));
    assert_eq!(LaneStates::UNKNOWN.readiness(Lane::Requests, EPOCH), Readiness::Wait(Wait::Channel));
}

/// The channels stay live for the rest of the life of an old token. A request would wait
/// for the plugin and fail, so the store waits for the credential.
#[test]
fn a_failed_credential_holds_each_request_and_a_refresh_of_a_valid_token_does_not() {
    let live = [ChannelState::Live];
    let lanes = |state| LaneStates::of(&status(&live, &live, Some(state)));
    let failed =
        CredentialState::Failed { failure: CredentialFailure::Refused, failures: 1, next_attempt: EPOCH + SECOND };
    let again = CredentialState::Running {
        reason: RunReason::Retry,
        since: EPOCH,
        last_failure: Some(CredentialFailure::Stopped),
    };
    for state in [failed, again] {
        let lanes = lanes(state);
        assert!(lanes.credential_failed);
        assert_eq!(lanes.readiness(Lane::Requests, EPOCH), Readiness::Wait(Wait::Credential));
    }
    let refresh = CredentialState::Running { reason: RunReason::Expiry, since: EPOCH, last_failure: None };
    for state in [refresh, CredentialState::Valid { until: None }] {
        assert_eq!(lanes(state), LIVE);
    }
}

#[test]
fn a_throttle_holds_a_live_lane_until_its_time() {
    let mut throttled = status(&[ChannelState::Live], &[ChannelState::Live], None);
    throttled.throttle = ThrottleStatus { until: Some(EPOCH + 5 * SECOND), answers: 1 };
    let lanes = LaneStates::of(&throttled);
    assert_eq!(lanes.readiness(Lane::Requests, EPOCH), Readiness::Throttled(EPOCH + 5 * SECOND));
    // The connection layer tells no end of a throttle, so a time in the past is over.
    assert_eq!(lanes.readiness(Lane::Requests, EPOCH + 5 * SECOND), Readiness::Ready);
    let lost = LaneStates { watches: Path::NoChannel, ..lanes };
    assert_eq!(lost.readiness(Lane::Watches, EPOCH), Readiness::Wait(Wait::Channel), "the channel comes first");
}

#[tokio::test]
async fn a_reader_hears_a_lost_lane_and_a_connection_that_returns() {
    let states = watch::Sender::new(LIVE);
    let mut lanes = Lanes::new(states.subscribe());
    assert!(lanes.lost(Lane::Watches).now_or_never().is_none());
    assert!(lanes.returned().now_or_never().is_none(), "lanes that stay live did not return");

    states.send_replace(LaneStates { requests: Path::NoChannel, ..LIVE });
    assert!(lanes.lost(Lane::Watches).now_or_never().is_none(), "the lane for watches is live");
    assert!(lanes.lost(Lane::Requests).now_or_never().is_some());
    {
        let returned = lanes.returned();
        tokio::pin!(returned);
        assert!(returned.as_mut().now_or_never().is_none(), "one lane is down");
        states.send_replace(LIVE);
        assert!(returned.now_or_never().is_some());
    }
    // The task of the cluster is gone: a loop must end and not wait for it.
    drop(states);
    assert!(lanes.changed().await.is_err());
    assert!(lanes.lost(Lane::Watches).now_or_never().is_none());
}
