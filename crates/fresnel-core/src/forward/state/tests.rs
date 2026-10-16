use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use fresnel_net::{ChannelStatus, ConnectStatus, GroupStatus, ThrottleStatus, UpgradeStatus};

use super::*;
use crate::forward::resolve::TargetPod;
use crate::store::ObjectKey;

const EPOCH: SystemTime = SystemTime::UNIX_EPOCH;

fn channel(state: ChannelState) -> ChannelStatus {
    ChannelStatus {
        state,
        since: EPOCH,
        streams_open: 0,
        stream_budget: 70,
        server_stream_limit: Some(100),
        ping_time: None,
        last_frame_age: None,
        connect_attempts: 1,
        connections: 1,
        last_error: Some("connect: no answer in 10s".to_owned()),
    }
}

/// A cluster whose three channels have this state.
fn cluster(state: ChannelState) -> ClusterStatus {
    let group = || GroupStatus { channels: vec![channel(state.clone())], waiting: 0 };
    ClusterStatus {
        name: "ams".to_owned(),
        connect: ConnectStatus {
            addresses: Vec::new(),
            resolve_error: None,
            connect_time: None,
            hedge_delay: Duration::from_millis(200),
            connects: 1,
            attempts: 1,
        },
        outage: None,
        watches: group(),
        requests: group(),
        logs: group(),
        credential: None,
        upgrade: UpgradeStatus::default(),
        throttle: ThrottleStatus::default(),
    }
}

fn pod() -> Resolution {
    let key = ObjectKey::new("shop", "web-1");
    Resolution::Pod(TargetPod { pod: key, uid: Arc::from("u1"), port: 8080, ready: true })
}

/// The facts of a forward with a listener, a live path and a pod.
fn active() -> Facts {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, 8080));
    Facts {
        listener: Listener::Bound(vec![address]),
        path: Ok(Path::Up),
        was_up: true,
        resolution: Some(pod()),
        failures: (0, None),
        refused: None,
    }
}

#[test]
fn the_path_comes_from_the_channels_with_the_attempt_and_the_time_of_the_next_one() {
    assert_eq!(Path::of(&cluster(ChannelState::Live), None, false), Path::Up);
    let first = cluster(ChannelState::Connecting { attempt: 1 });
    assert_eq!(Path::of(&first, None, false), Path::First);
    let lost = Path::of(&first, None, true);
    let reason = "the cluster has no connection".to_owned();
    assert_eq!(lost, Path::Down { attempt: Some(1), next_retry: None, reason }, "a connect after a loss");
    let pause = ChannelState::Backoff { failures: 2, pause: Duration::from_secs(3) };
    let reason = Some("requests: connect: no answer in 10s".to_owned());
    let next_retry = Some(EPOCH + Duration::from_secs(3));
    let down = Path::Down { attempt: Some(3), next_retry, reason: reason.clone().unwrap_or_default() };
    assert_eq!(Path::of(&cluster(pause), reason.clone(), false), down);
    let again = Path::of(&cluster(ChannelState::Connecting { attempt: 4 }), reason, false);
    assert!(matches!(again, Path::Down { attempt: Some(4), next_retry: None, .. }), "{again:?}");
}

#[test]
fn each_set_of_facts_gives_its_state() {
    assert_eq!(active().state(), ForwardState::Active);
    assert!(active().opens());

    let mut facts = active();
    facts.listener = Listener::Failed("the local port 8080 is in use".to_owned());
    assert_eq!(facts.state(), ForwardState::Failed("the local port 8080 is in use".to_owned()));
    assert!(!facts.opens());
    facts.listener = Listener::Binding;
    assert_eq!(facts.state(), ForwardState::Starting("the local port binds".to_owned()));

    let down = Path::Down { attempt: Some(2), next_retry: None, reason: "silent".to_owned() };
    let reconnecting = Facts { path: Ok(down), ..active() };
    let state = ForwardState::Reconnecting { attempt: Some(2), next_retry: None, reason: "silent".to_owned() };
    assert_eq!((reconnecting.state(), reconnecting.opens()), (state, false));
    assert_eq!(Facts { path: Ok(Path::First), was_up: false, ..active() }.state().name(), "starting");

    let no_cluster = |was_up| Facts { path: Err("ams connects".to_owned()), was_up, ..active() };
    assert_eq!(no_cluster(false).state(), ForwardState::Starting("ams connects".to_owned()));
    assert_eq!(no_cluster(true).state(), ForwardState::Degraded("ams connects".to_owned()));

    let no_pod = Facts { resolution: Some(Resolution::NoPod("no pod runs".to_owned())), ..active() };
    assert_eq!((no_pod.state(), no_pod.opens()), (ForwardState::Degraded("no pod runs".to_owned()), false));
    let loading = Facts { resolution: Some(Resolution::NotKnown("the pods load".to_owned())), ..active() };
    assert_eq!(loading.state(), ForwardState::Starting("the pods load".to_owned()));
    assert_eq!(Facts { resolution: None, ..active() }.state().name(), "starting");
}

/// Each of the last tunnels failed: the state tells it, and the gate stays open, so one
/// good tunnel ends it (port forward prototype, R10).
#[test]
fn tunnels_that_fail_in_a_row_degrade_the_state_and_a_refusal_fails_it() {
    let failing = |count| Facts { failures: (count, Some("connection refused".to_owned())), ..active() };
    assert_eq!(failing(FAILURES - 1).state(), ForwardState::Active);
    let text = format!("each of the last {FAILURES} tunnels failed: connection refused");
    assert_eq!((failing(FAILURES).state(), failing(FAILURES).opens()), (ForwardState::Degraded(text), true));

    let refused = Facts { refused: Some("the server refuses the port forward: forbidden".to_owned()), ..active() };
    assert_eq!(refused.state().to_string(), "failed: the server refuses the port forward: forbidden");
    assert!(!refused.opens());
}

#[test]
fn a_new_time_of_the_next_attempt_is_the_same_step() {
    let at = |seconds| ForwardState::Reconnecting {
        attempt: Some(seconds),
        next_retry: Some(EPOCH + Duration::from_secs(u64::from(seconds))),
        reason: "silent".to_owned(),
    };
    assert!(at(1).same_step(&at(2)));
    assert!(!at(1).same_step(&ForwardState::Active));
    assert_eq!(at(3).to_string(), "reconnecting, attempt 3: silent");
}
