use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

use fresnel_core::cluster::ClusterId;
use fresnel_core::forward::{Bind, Bytes, Connections, ForwardRule, LocalPort, TargetPod};
use fresnel_core::kinds::Kind;
use fresnel_core::store::ObjectKey;

use super::*;
use crate::testing::TestTasks;

const SECOND: Duration = Duration::from_secs(1);

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + 1_800_000_000 * SECOND
}

fn view(state: ForwardState) -> ForwardView {
    let rule = ForwardRule {
        cluster: ClusterId::new("ams"),
        kind: Kind::Service,
        key: ObjectKey::new("shop", "web"),
        port: 80,
        local: LocalPort::Fixed(8080),
        bind: Bind::Localhost,
    };
    ForwardView::new(rule, state, now())
}

#[test]
fn the_state_of_a_reconnect_tells_its_attempt_and_the_time_to_the_next_one() {
    let clock = Clock::fixed(now());
    let reconnecting = |next_retry| ForwardState::Reconnecting {
        attempt: Some(3),
        next_retry,
        reason: "requests: connect: no answer in 10s".to_owned(),
    };
    let (text, tone) = state(&view(reconnecting(Some(now() + 4 * SECOND))), &clock);
    assert_eq!(
        (text.as_str(), tone),
        ("reconnecting, attempt 3, next in 4 s: requests: connect: no answer in 10s", Tone::Warn)
    );
    assert!(state(&view(reconnecting(None)), &clock).0.starts_with("reconnecting, attempt 3, next now: "));
    assert_eq!(state(&view(ForwardState::Active), &clock), ("active".to_owned(), Tone::Ok));
    let short = short_state(&view(reconnecting(Some(now() + 4 * SECOND))), &clock);
    assert_eq!(short, ("reconnecting, next in 4 s".to_owned(), Tone::Warn), "the list has no reason");
    let failed = ForwardState::Failed("the local port 8080 is in use".to_owned());
    assert_eq!(state(&view(failed.clone()), &clock), ("failed: the local port 8080 is in use".to_owned(), Tone::Error));
    assert_eq!(short_state(&view(failed), &clock), ("failed".to_owned(), Tone::Error));
}

#[test]
fn the_pod_the_addresses_the_counts_the_data_and_the_last_error_read_as_short_texts() {
    let clock = Clock::fixed(now());
    let mut shown = view(ForwardState::Active);
    assert_eq!((pod(&shown).0, addresses(&shown), first_address(&shown)), ("–".into(), "–".into(), "–".into()));
    shown.target = Some(TargetPod { pod: ObjectKey::new("shop", "web-1"), uid: "u1".into(), port: 8080, ready: false });
    shown.addresses =
        ["127.0.0.1:8080", "[::1]:8080"].map(|text| text.parse::<SocketAddr>().expect("an address")).into();
    shown.connections = Connections { open: 2, total: 1400, reset: 1 };
    shown.bytes = Bytes { sent: 12 * 1024 * 1024, received: 300 * 1024 };
    shown.data = TargetData::Old { kind: Kind::EndpointSlice, since: now() - 48 * SECOND };
    shown.last_error = Some((now() - 12 * SECOND, "a tunnel broke: silent".to_owned()));
    assert_eq!(pod(&shown), ("web-1:8080, not ready".to_owned(), Tone::Warn));
    assert_eq!(
        (addresses(&shown).as_str(), first_address(&shown).as_str()),
        ("127.0.0.1:8080, [::1]:8080", "127.0.0.1:8080")
    );
    assert_eq!(connections(&shown), "2 open, 1,400 in all, 1 reset");
    assert_eq!(bytes(&shown), "12Mi sent, 300Ki received");
    assert_eq!(data(&shown, &clock).0, "old for 48 s: the EndpointSlices are not live");
    assert_eq!(last_error(&shown, &clock).0, "12 s ago: a tunnel broke: silent");
}

#[test]
fn the_summary_counts_the_forwards_of_each_state() {
    let tasks = TestTasks::new();
    let row = |id, state| ForwardRow { id, saved: false, view: tasks.held(view(state)) };
    let rows = [row(1, ForwardState::Active), row(2, ForwardState::Stopped), row(3, ForwardState::Active)];
    assert_eq!(summary(&rows), "3 forwards: 2 active, 1 stopped");
    assert_eq!(summary(&rows[..1]), "1 forward: 1 active");
}
