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
