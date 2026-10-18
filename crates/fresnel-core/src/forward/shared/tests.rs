use std::sync::Arc;

use fresnel_net::ErrorClass;
use tokio::time::advance;

use super::*;
use crate::store::ObjectKey;

fn shared() -> Shared {
    let times = [Duration::from_secs(5), Duration::from_secs(1), Duration::from_millis(500)];
    Shared::new(Journal::new(100, WallClock::system()), times, WallClock::system())
}

fn target() -> TargetPod {
    TargetPod { pod: ObjectKey::new("shop", "web-1"), uid: Arc::from("u1"), port: 8080, ready: true }
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
    shared.report(Outcome::Broke { text: "the node refused a tunnel: connection refused".to_owned(), path: false });
    shared.report(Outcome::Opened);
    assert_eq!(read(&shared).0, 3, "an open after a refusal");
    shared.report(Outcome::Carried);
    assert_eq!(read(&shared), (0, None));
    shared.report(Outcome::Stalled { target: target() });
    let text = shared.reports().last_error.as_ref().map(|(_, text)| text.clone());
    assert_eq!(text.as_deref(), Some("the open of a tunnel to pod web-1 had no answer in 5.0 s"));
}

/// A failure counts for the window after it: an idle forward shows no fault of long ago.
/// Each new failure moves the end of the run.
#[tokio::test(start_paused = true)]
async fn the_failures_end_a_window_after_the_last_one() {
    let shared = shared();
    let broke = || Outcome::Broke { text: "a tunnel broke: reset".to_owned(), path: true };
    shared.report(broke());
    shared.report(Outcome::NoTunnel("no tunnel".to_owned()));
    advance(WINDOW - Duration::from_secs(1)).await;
    shared.report(broke());
    assert_eq!(read(&shared), (2, Some("a tunnel broke: reset".to_owned())));
    advance(Duration::from_secs(1)).await;
    assert_eq!(read(&shared), (2, None), "the connection with no tunnel is older than the window");
    advance(WINDOW - Duration::from_secs(2)).await;
    assert_eq!(read(&shared).0, 2);
    advance(Duration::from_secs(1)).await;
    assert_eq!(read(&shared), (0, None));
}
