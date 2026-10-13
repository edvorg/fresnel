//! The mode `Freeze` and the silent connections: open connections that carry nothing.

use fresnel_testkit::{ProxyCounters, ProxyMode};
use tokio::io::AsyncWriteExt;
use tokio::time::timeout;

use super::{Bed, QUIET, end_of, in_time, is_quiet, is_reset, receive};

#[tokio::test]
async fn freeze_stops_all_bytes_and_each_close_until_the_mode_changes() {
    let bed = Bed::start().await;
    let (mut client, mut server) = bed.pair().await;
    let (closing_client, mut closing_server) = bed.pair().await;
    bed.proxy.set(ProxyMode::Freeze);

    client.write_all(b"up").await.expect("the kernel takes the bytes");
    server.write_all(b"down").await.expect("the kernel takes the bytes");
    drop(closing_client);
    assert!(is_quiet(&mut server).await && is_quiet(&mut client).await, "no byte passes");
    assert!(is_quiet(&mut closing_server).await, "no close passes");
    let _new_client = bed.connect().await;
    assert!(timeout(QUIET, bed.upstream.accept()).await.is_err(), "a new connection gets no answer");

    bed.proxy.set(ProxyMode::Pass);
    assert_eq!(receive(&mut server, 2).await, b"up", "the bytes of the freeze arrive");
    assert_eq!(receive(&mut client, 4).await, b"down");
    assert!(matches!(end_of(&mut closing_server).await, Ok(0)), "the close arrives");
    drop(bed.accept().await);
}

/// A break of the network for minutes: the client gives up each connect attempt at its
/// time limit. At the end of the freeze, only the attempt that still waits gets its server.
#[tokio::test]
async fn a_new_connection_that_its_client_closes_during_a_freeze_never_reaches_the_server() {
    let bed = Bed::start().await;
    bed.proxy.set(ProxyMode::Freeze);
    for _ in 0..3 {
        let mut given_up = bed.connect().await;
        given_up.write_all(b"hello of a client that gives up").await.expect("the kernel takes the bytes");
    }
    let mut patient = bed.connect().await;
    patient.write_all(b"early ").await.expect("the kernel takes the bytes");
    // A connection counts as given up before it leaves the count of the open ones.
    let held = |counters: &ProxyCounters| counters.gave_up == 3 && counters.open == 1;
    let counters = in_time(bed.proxy.wait_for(held)).await;
    assert_eq!(counters.accepted, 4, "the proxy holds one connection of the four");

    bed.proxy.set(ProxyMode::Pass);
    let mut server = bed.accept().await;
    patient.write_all(b"bytes").await.expect("write");
    assert_eq!(receive(&mut server, 11).await, b"early bytes", "the bytes of the freeze come first");
    assert!(timeout(QUIET, bed.upstream.accept()).await.is_err(), "no other connection reaches the server");
}

#[tokio::test]
async fn silent_connections_carry_nothing_for_all_time_and_new_connections_work() {
    let bed = Bed::start().await;
    let (mut client, mut server) = bed.pair().await;
    let (closing_client, mut closing_server) = bed.pair().await;
    bed.proxy.silence_open();

    client.write_all(b"up").await.expect("the kernel takes the bytes");
    server.write_all(b"down").await.expect("the kernel takes the bytes");
    drop(closing_client);
    assert!(is_quiet(&mut server).await && is_quiet(&mut client).await, "no byte passes");
    assert!(is_quiet(&mut closing_server).await, "no close passes");

    let (mut new_client, mut new_server) = bed.pair().await;
    new_client.write_all(b"request").await.expect("write");
    assert_eq!(receive(&mut new_server, 7).await, b"request", "a new connection works");
    new_server.write_all(b"answer").await.expect("write");
    assert_eq!(receive(&mut new_client, 6).await, b"answer");

    bed.proxy.set(ProxyMode::Freeze);
    bed.proxy.set(ProxyMode::Pass);
    assert!(is_quiet(&mut server).await && is_quiet(&mut client).await, "a mode change does not help");
    let counters = in_time(bed.proxy.wait_for(|counters| counters.accepted == 3)).await;
    assert_eq!(counters.open, 3, "a silent connection stays open");
}

/// A second fault during the first: a reset of all connections while the proxy is frozen.
#[tokio::test]
async fn a_reset_during_a_freeze_reaches_each_connection_and_the_freeze_stays() {
    let bed = Bed::start().await;
    let (mut client, mut server) = bed.pair().await;
    bed.proxy.set(ProxyMode::Freeze);
    let mut waiting = bed.connect().await;
    client.write_all(b"lost").await.expect("the kernel takes the bytes");
    in_time(bed.proxy.wait_for(|counters| counters.open == 2)).await;

    in_time(bed.proxy.reset_all()).await;
    for socket in [&mut client, &mut server, &mut waiting] {
        assert!(is_reset(&end_of(socket).await), "a frozen connection gets the reset");
    }
    assert_eq!(bed.proxy.counters(), ProxyCounters { accepted: 2, reset: 2, ..ProxyCounters::default() });

    let mut late_client = bed.connect().await;
    assert!(timeout(QUIET, bed.upstream.accept()).await.is_err(), "the reset does not end the freeze");
    bed.proxy.set(ProxyMode::Pass);
    let mut late_server = bed.accept().await;
    late_client.write_all(b"again").await.expect("write");
    assert_eq!(receive(&mut late_server, 5).await, b"again", "a connection after the two faults works");
}
