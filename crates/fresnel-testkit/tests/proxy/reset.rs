//! The reset of all connections: one time on the order of a test, and again and again.

use std::time::{Duration, Instant};

use fresnel_testkit::ProxyMode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

use super::{Bed, LIMIT, QUIET, end_of, in_time, is_quiet, is_reset, receive};

#[tokio::test]
async fn reset_all_gives_each_side_of_each_connection_a_reset() {
    let bed = Bed::start().await;
    let mut pairs = Vec::new();
    for _ in 0..3 {
        pairs.push(bed.pair().await);
    }
    in_time(bed.proxy.reset_all()).await;
    let counters = bed.proxy.counters();
    assert_eq!((counters.open, counters.reset), (0, 3), "the resets are complete at the return");
    for (client, server) in &mut pairs {
        assert!(is_reset(&end_of(client).await), "the client gets a reset");
        assert!(is_reset(&end_of(server).await), "the server gets a reset");
    }

    let (mut client, mut server) = bed.pair().await;
    client.write_all(b"again").await.expect("write");
    assert_eq!(receive(&mut server, 5).await, b"again", "a connection after the reset works");
}

/// A reset with data on the way is not reliable on the loopback of macOS when the sender
/// closes at once. The client then waits with no end, which is a different fault.
#[tokio::test]
async fn reset_all_during_a_transfer_reaches_the_client_each_time() {
    let bed = Bed::start().await;
    let chunk = vec![7_u8; 64 * 1024];
    for round in 0..40 {
        let (mut client, mut server) = bed.pair().await;
        let download = async {
            let mut buffer = vec![0_u8; 64 * 1024];
            loop {
                match timeout(LIMIT, client.read(&mut buffer)).await {
                    Ok(Ok(0)) => return "the end of data",
                    Ok(Ok(_)) => {}
                    Ok(Err(_)) => return "a reset",
                    Err(_) => return "nothing",
                }
            }
        };
        let upload_and_reset = async {
            for _ in 0..(20 + round % 30) {
                if server.write_all(&chunk).await.is_err() {
                    break;
                }
            }
            bed.proxy.reset_all().await;
        };
        let (got, ()) = tokio::join!(download, upload_and_reset);
        assert_eq!(got, "a reset", "round {round}");
    }
}

#[tokio::test]
async fn reset_each_resets_all_connections_again_and_again_until_the_mode_changes() {
    let period = Duration::from_millis(100);
    let bed = Bed::start().await;
    let started = Instant::now();
    bed.proxy.set(ProxyMode::ResetEach(period));
    for _ in 0..4 {
        let (mut client, mut server) = bed.pair().await;
        assert!(is_reset(&end_of(&mut client).await) && is_reset(&end_of(&mut server).await));
    }
    assert!(started.elapsed() >= 4 * period, "4 resets came in {:?}", started.elapsed());
    assert_eq!(bed.proxy.counters().reset, 4);

    bed.proxy.set(ProxyMode::Pass);
    let (mut client, mut server) = bed.pair().await;
    assert!(QUIET > period, "the quiet time is longer than one period");
    assert!(is_quiet(&mut client).await && is_quiet(&mut server).await, "no reset after the mode change");
}
