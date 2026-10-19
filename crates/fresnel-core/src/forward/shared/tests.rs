use std::sync::Arc;

use fresnel_net::ErrorClass;
use tokio::time::advance;

use super::*;
use crate::store::ObjectKey;

/// A wall clock that paused time moves.
fn clock() -> WallClock {
    let start = Instant::now();
    WallClock::new(move || SystemTime::UNIX_EPOCH + start.elapsed())
}

fn shared() -> Shared {
    let times = [Duration::from_secs(5), Duration::from_secs(1), Duration::from_millis(500)];
    Shared::new(Journal::new(100, WallClock::system()), times, clock())
}

fn target() -> TargetPod {
    TargetPod { pod: ObjectKey::new("shop", "web-1"), uid: Arc::from("u1"), port: 8080, ready: true }
}

fn broke(pod: &str) -> Outcome {
    Outcome::Broke { text: format!("a tunnel to {pod} broke: reset"), path: true, pod: Arc::from(pod) }
}

/// The facts that the task reads: the failures in a row, and the reason of the last
/// connection with no tunnel.
fn read(shared: &Shared) -> (u32, Option<String>) {
    let mut reports = shared.reports();
    reports.expire(Instant::now());
    (reports.failures, reports.no_tunnel.as_ref().map(|(_, reason)| reason.clone()))
}

/// A stalled open is a failure, and a connection with no tunnel keeps the reason of its last
/// open. An open ends the connection with no tunnel, and the first bytes of a pod end the
/// failures: a long tunnel that works does not have to end first, and a tunnel that opens and
/// then fails, as each one to a port that refuses connections, ends no run.
#[tokio::test(start_paused = true)]
async fn a_stalled_open_and_a_connection_with_no_tunnel_count_until_the_first_bytes_of_a_pod() {
    let shared = shared();
    shared.report(Outcome::Stalled { target: target() });
    let network = CoreError::new(ErrorClass::Network, "reset");
    shared.report(Outcome::OpenFailed { target: target(), error: network });
    shared.report(Outcome::NoTunnel("a local connection got no tunnel before the end of its wait: a reset".to_owned()));
    let reason = "a tunnel to pod web-1 did not open: reset".to_owned();
    assert_eq!(read(&shared), (2, Some(reason)));
    assert!(std::mem::take(&mut shared.reports().retry), "a stall asks the channels to test their path");
    let text = shared.reports().last_error.as_ref().map(|(_, text)| text.clone());
    assert_eq!(text.as_deref(), Some("a local connection got no tunnel before the end of its wait: a reset"));

    shared.report(Outcome::Opened);
    assert_eq!(read(&shared), (2, None), "a tunnel opened");
    let refused = "the node refused a tunnel: connection refused".to_owned();
    shared.report(Outcome::Broke { text: refused, path: false, pod: Arc::from("u1") });
    shared.report(Outcome::Opened);
    assert_eq!(read(&shared).0, 3, "an open after a refusal");
    shared.report(Outcome::Carried);
    assert_eq!(read(&shared), (0, None));
    shared.report(Outcome::Stalled { target: target() });
    let text = shared.reports().last_error.as_ref().map(|(_, text)| text.clone());
    assert_eq!(text.as_deref(), Some("the open of a tunnel to pod web-1 had no answer in 5.0 s"));
}

/// The failures follow the last connections, not a clock: a forward to a port that refuses
/// each connection stays degraded also when its clients come minutes apart, and the last
/// failure keeps its time for the age that a view shows.
#[tokio::test(start_paused = true)]
async fn the_failures_hold_until_the_next_connection_also_long_after_the_last_one() {
    let shared = shared();
    for _ in 0..3 {
        shared.report(broke("u1"));
        advance(Duration::from_secs(40)).await;
    }
    shared.report(Outcome::NoTunnel("no tunnel".to_owned()));
    advance(Duration::from_secs(600)).await;
    assert_eq!(read(&shared), (3, Some("a tunnel to u1 broke: reset".to_owned())));
    let reports = shared.reports();
    let last = reports.last_failure.as_ref().map(|(at, _)| *at);
    assert_eq!(last, Some(SystemTime::UNIX_EPOCH + Duration::from_secs(80)), "the time of the last failure");
    let no_tunnel = reports.no_tunnel.as_ref().map(|(at, _)| *at);
    assert_eq!(no_tunnel, Some(SystemTime::UNIX_EPOCH + Duration::from_secs(120)));
}

/// A new target pod ends the failures, and a failure of an other pod, as a tunnel to the old
/// pod that ends after the change, is no failure of the target.
#[tokio::test(start_paused = true)]
async fn a_new_target_pod_ends_the_failures_and_a_failure_of_an_other_pod_does_not_count() {
    let shared = shared();
    assert!(!shared.reports().aim(&Arc::from("u1")), "the first target");
    shared.report(broke("u1"));
    shared.report(broke("u1"));
    shared.report(Outcome::NoTunnel("no tunnel".to_owned()));
    assert!(!shared.reports().aim(&Arc::from("u1")), "the same target");
    assert_eq!(read(&shared).0, 2);
    assert!(shared.reports().aim(&Arc::from("u2")), "a new target");
    assert_eq!(read(&shared), (0, None));
    shared.report(broke("u1"));
    let stalled = TargetPod { uid: Arc::from("u1"), ..target() };
    shared.report(Outcome::Stalled { target: stalled });
    assert_eq!(read(&shared).0, 0, "failures of the old pod");
    shared.report(broke("u2"));
    assert_eq!(read(&shared).0, 1);
}
