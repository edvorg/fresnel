//! The breaks of plan section 15, M5: a network that goes away and comes back, and a wake,
//! with six forwards on one cluster and the default times of the pings.

use std::sync::atomic::Ordering;
use std::time::SystemTime;

use fresnel_testkit::ProxyMode;
use tokio::time::Instant;

use super::*;
use crate::common::in_time;

/// Six forwards of the pod `web-1`, each on a port of its own, once each one is active.
async fn six_forwards(bed: &Bed) -> Vec<(ForwardRule, SocketAddr)> {
    let mut forwards = Vec::new();
    for port in 8081..=8086 {
        let rule = rule(Kind::Pod, "web-1", port);
        bed.start_forward(&rule);
        forwards.push(rule);
    }
    let mut active = Vec::new();
    for rule in forwards {
        let address = bed.active(&rule).await;
        assert_eq!(echo(address, b"before").await, Ok(()), "{rule}");
        active.push((rule, address));
    }
    active
}

/// Waits until each forward can carry bytes again.
async fn until_each_echoes(forwards: &[(ForwardRule, SocketAddr)]) {
    for (rule, address) in forwards {
        in_time(&format!("the bytes of {rule}"), async {
            while echo(*address, b"after").await.is_err() {
                tokio::task::yield_now().await;
            }
        })
        .await;
    }
}

/// Plan section 15, M5: six forwards on one cluster each show a silent path in 20 s or less,
/// the local port stays open during the break, and each forward works again 10 s or less
/// after the network returns. In the prototype, the sixth forward showed it after 39 s.
#[tokio::test(flavor = "multi_thread")]
async fn six_forwards_show_a_silent_path_in_20_s_and_recover_in_10_s_after_the_network_returns() {
    let bed = Bed::start("forward-break").await;
    let forwards = six_forwards(&bed).await;

    // The network is gone: open connections carry nothing, and new ones get no answer.
    let silent = SystemTime::now();
    bed.proxy.set(ProxyMode::Freeze);
    for (rule, _) in &forwards {
        let row = bed.until(rule, "the state reconnecting", |view| view.state.name() == "reconnecting").await;
        let after = row.view.since.duration_since(silent).unwrap_or_default();
        println!("{rule}: reconnecting {after:?} after the start of the silence");
        assert!(after <= 20 * SECOND, "{rule} showed the break after {after:?}");
    }

    // A local connection in the break: the port is open, and the connection waits 5 s for
    // a tunnel, then gets a reset.
    let (rule, address) = &forwards[0];
    let connected = Instant::now();
    assert_eq!(end_seen(*address).await, Ok("reset"));
    let waited = connected.elapsed();
    assert!((5 * SECOND..10 * SECOND).contains(&waited), "the reset came after {waited:?}");
    let row = bed.until(rule, "the reset", |view| view.connections.reset == 1).await;
    let error = row.view.last_error.as_ref().map(|(_, text)| text.as_str());
    assert_eq!(error, Some("a local connection got no tunnel in 5.0 s: a reset"));

    let back = Instant::now();
    bed.proxy.set(ProxyMode::Pass);
    until_each_echoes(&forwards).await;
    println!("each forward carried bytes {:?} after the network returned", back.elapsed());
    assert!(back.elapsed() <= 10 * SECOND, "the forwards worked again after {:?}", back.elapsed());
    for (rule, _) in &forwards {
        bed.until(rule, "the state active", |view| view.state.name() == "active").await;
    }
}

/// Plan section 15, M5: a forward recovers 10 s or less after a wake. In the sleep, each
/// connection went silent, and new connections work: the wake ends the waits, and a
/// forward does not wait for the pings of the old connections.
#[tokio::test(flavor = "multi_thread")]
async fn each_forward_works_again_in_10_s_after_a_wake() {
    let bed = Bed::start("forward-wake").await;
    let forwards = six_forwards(&bed).await;

    bed.proxy.silence_open();
    let woke = Instant::now();
    bed.app.woke.store(true, Ordering::Relaxed);
    until_each_echoes(&forwards).await;
    println!("each forward carried bytes {:?} after the wake", woke.elapsed());
    assert!(woke.elapsed() <= 10 * SECOND, "the forwards worked again after {:?}", woke.elapsed());
    let texts =
        bed.app.until_texts("app", |texts| texts.iter().any(|text| text.starts_with("the computer slept"))).await;
    assert!(texts.iter().any(|text| text.starts_with("the computer slept for 60 s")), "{texts:?}");
}
