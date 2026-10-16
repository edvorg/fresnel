//! The breaks of plan section 15, M5: a network that goes away and comes back, and a wake,
//! with six forwards on one cluster and the default times of the pings.

use std::sync::atomic::Ordering;
use std::time::SystemTime;

use fresnel_core::forward::ForwardSettings;
use fresnel_testkit::ProxyMode;
use tokio::time::Instant;

use super::*;
use crate::common::in_time;

/// Settings of a test with short pings: each channel and each tunnel pings after 1 s with no
/// frame and breaks with no answer in 1 s, so a silent path shows in about 2 s.
fn short_pings(wait: Duration) -> CoreSettings {
    let mut core = settings();
    for lane in [&mut core.net.watches, &mut core.net.requests, &mut core.net.logs] {
        (lane.keepalive_interval, lane.keepalive_timeout) = (SECOND, SECOND);
    }
    let socket = &mut core.net.upgrade.socket;
    (socket.ping_after, socket.answer_limit) = (SECOND, SECOND);
    core.forwards = ForwardSettings { wait, ..ForwardSettings::default() };
    core
}

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

/// Plan section 12.3: an abnormal end always gives the local client an error, never a silent
/// stop. The open connections go silent while a client waits for the next bytes: its tunnel
/// breaks on its ping, and the client gets a reset.
#[tokio::test(flavor = "multi_thread")]
async fn a_tunnel_that_goes_silent_gives_its_client_a_reset() {
    let bed = Bed::start_with("forward-silent-tunnel", short_pings(5 * SECOND)).await;
    let pod = rule(Kind::Pod, "web-1", 8080);
    bed.start_forward(&pod);
    let address = bed.active(&pod).await;
    let mut client = TcpStream::connect(address).await.expect("a connect");
    client.write_all(b"one").await.expect("a write");
    let mut back = [0_u8; 3];
    client.read_exact(&mut back).await.expect("the bytes back");

    bed.proxy.silence_open();
    let end = timeout(30 * SECOND, client.read(&mut back)).await;
    assert!(matches!(end, Ok(Err(_))), "a reset, not an end of the data: {end:?}");
    let row = bed.until(&pod, "the reset", |view| view.connections.reset == 1).await;
    let error = row.view.last_error.as_ref().map(|(_, text)| text.as_str()).unwrap_or_default();
    assert!(error.starts_with("a tunnel broke: "), "{error}");
}

/// Plan section 12.3: a local connection that arrives in a break waits for a tunnel, and
/// gets one when the path returns before the end of its wait.
#[tokio::test(flavor = "multi_thread")]
async fn a_local_connection_in_a_break_gets_its_tunnel_when_the_path_returns_in_its_wait() {
    let bed = Bed::start_with("forward-wait", short_pings(30 * SECOND)).await;
    let pod = rule(Kind::Pod, "web-1", 8080);
    bed.start_forward(&pod);
    let address = bed.active(&pod).await;
    bed.proxy.set(ProxyMode::Freeze);
    bed.until(&pod, "the state reconnecting", |view| view.state.name() == "reconnecting").await;
    let mut client = TcpStream::connect(address).await.expect("the port stays open");
    client.write_all(b"in the break").await.expect("a write");
    bed.until(&pod, "a connection that waits", |view| view.connections.open == 1).await;

    bed.proxy.set(ProxyMode::Pass);
    let mut back = [0_u8; 12];
    let read = timeout(30 * SECOND, client.read_exact(&mut back)).await;
    assert!(matches!(read, Ok(Ok(_))), "{read:?}");
    assert_eq!(&back, b"in the break");
    let row = bed.until(&pod, "the counts", |view| view.bytes.received == 12).await;
    assert_eq!(row.view.connections.reset, 0);
}
