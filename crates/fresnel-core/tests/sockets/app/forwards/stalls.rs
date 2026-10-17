//! Plan section 12.3: a forward whose new connections get no tunnel is never "active". Five
//! fast soaks found the forward "active" with no page for 20 s and more: after a reset storm
//! and a restart of the API server, which then held the answers to the upgrade requests.

use std::time::SystemTime;

use fresnel_testkit::{Fault, ProxyMode};
use tokio::time::{Instant, sleep};

use super::*;
use crate::common::in_time;
use crate::gaps::longest_hidden_failure;

/// The path of each port forward of the pods of the namespace `shop`.
const FORWARDS: &str = "/api/v1/namespaces/shop/pods/";

/// The page of a soak check: the bytes back through the local port in 4 s.
async fn fetch(address: SocketAddr) -> bool {
    timeout(4 * SECOND, echo(address, b"page")).await.is_ok_and(|echoed| echoed.is_ok())
}

/// An API server that holds each upgrade request has a live path, so only the opens can
/// tell. The connection gets a reset at the end of its wait, and the forward shows that
/// in the wait and 1 s, not after three opens of 10 s each. A node that answers HTTP 101
/// and starts no stream is the same. The first good tunnel ends the state.
#[tokio::test(flavor = "multi_thread")]
async fn tunnels_with_no_answer_degrade_the_forward_in_its_wait_and_a_good_tunnel_ends_it() {
    let bed = Bed::start("forward-stall").await;
    let pod = rule(Kind::Pod, "web-1", 8080);
    bed.start_forward(&pod);
    let address = bed.active(&pod).await;
    assert_eq!(echo(address, b"before").await, Ok(()));

    for (fault, what) in [(true, "held upgrade answers"), (false, "a node that starts no stream")] {
        if fault {
            bed.server.add_fault(Fault::no_answer().on_path(FORWARDS));
        } else {
            bed.server.mute_forwards(true);
        }
        let connected = (Instant::now(), SystemTime::now());
        assert_eq!(end_seen(address).await, Ok("reset"), "{what}");
        let reset = connected.0.elapsed();
        assert!((5 * SECOND..10 * SECOND).contains(&reset), "{what}: the reset came after {reset:?}");
        let row = bed.until(&pod, what, |view| view.state.name() == "degraded").await;
        let text = "degraded: the last local connection got no tunnel: the open of a tunnel to pod web-1 had no \
                    answer in 5.0 s";
        assert_eq!(row.view.state.to_string(), text, "{what}");
        let shown = row.view.since.duration_since(connected.1);
        assert!(shown.as_ref().is_ok_and(|shown| *shown <= 7 * SECOND), "{what}: degraded after {shown:?}");

        bed.server.clear_faults();
        bed.server.mute_forwards(false);
        assert_eq!(echo(address, b"after").await, Ok(()), "{what}");
        let row = bed.until(&pod, "the first good tunnel", |view| view.state.name() == "active").await;
        let error = row.view.last_error.as_ref().map(|(_, text)| text.as_str());
        assert_eq!(error, Some("a local connection got no tunnel before the end of its wait: a reset"));
    }
}

/// The faults of the fast soaks, while a client asks a page each second as the soak does: a
/// reset of each connection each 5 s, upgrade answers that the API server holds, and a
/// restart, which resets each connection, refuses for a time and then holds the upgrades.
/// No run of failed pages may last 10 s with the forward "active"; before the fix, a run
/// lasted for each time the server held the upgrades.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_of_a_forward_sees_a_state_for_each_run_of_failed_pages_in_the_faults_of_the_soak() {
    let bed = Bed::start("forward-soak-faults").await;
    let service = rule(Kind::Service, "web", 80);
    bed.start_forward(&service);
    let address = bed.active(&service).await;
    let active = || forward(&bed.app.core.view(), &service).is_some_and(|row| row.view.state.name() == "active");
    let faults = async {
        bed.proxy.set(ProxyMode::ResetEach(5 * SECOND));
        sleep(15 * SECOND).await;
        bed.proxy.set(ProxyMode::Pass);
        bed.server.add_fault(Fault::no_answer().on_path(FORWARDS).for_time(12 * SECOND));
        sleep(15 * SECOND).await;
        // The restart: each connection ends, a new one ends at once, and the server that
        // answers again holds the upgrades while its nodes return.
        bed.proxy.reset_all().await;
        bed.proxy.set(ProxyMode::ResetEach(SECOND / 5));
        sleep(2 * SECOND).await;
        bed.server.add_fault(Fault::no_answer().on_path(FORWARDS).for_time(12 * SECOND));
        bed.proxy.set(ProxyMode::Pass);
        sleep(15 * SECOND).await;
    };
    let (hidden, failed) = longest_hidden_failure(|| fetch(address), active, faults).await;
    println!("{failed} pages failed; the longest run with no state lasted {hidden:?}");
    assert!(failed >= 4, "the held upgrades failed pages: {failed}");
    assert!(hidden < 10 * SECOND, "the forward showed \"active\" for {hidden:?} of failed pages");
    in_time("a page after the faults", async {
        while !fetch(address).await {
            sleep(SECOND / 10).await;
        }
    })
    .await;
    bed.until(&service, "the state active", |view| view.state.name() == "active").await;
}
