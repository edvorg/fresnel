//! The reset of all connections: one time on the order of a test, and again and again.

use std::net::Ipv4Addr;
use std::pin::pin;
use std::time::{Duration, Instant};

use fresnel_testkit::ProxyMode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::yield_now;
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

/// The order can come before the task of a new connection ran one time. Such a connection
/// must close with a reset, not with an end of data, and the counter must show it.
#[tokio::test]
async fn reset_all_at_the_moment_of_a_connect_gives_a_reset_and_counts_it() {
    let bed = Bed::start().await;
    for round in 0..40 {
        let mut client = bed.connect().await;
        // A reset can be complete before the proxy takes the connection. The next one gets it.
        in_time(async {
            while bed.proxy.counters().reset == round {
                bed.proxy.reset_all().await;
            }
        })
        .await;
        assert!(is_reset(&end_of(&mut client).await), "round {round}: a reset and not an end of data");
        let counters = bed.proxy.counters();
        assert_eq!((counters.reset, counters.open), (round + 1, 0), "round {round}");
    }
}

/// A client that does not read never acknowledges the bytes that the proxy holds for it.
/// The reset must not wait for them with no end.
#[tokio::test]
async fn reset_all_reaches_a_client_that_does_not_read() {
    let bed = Bed::start().await;
    let (mut client, mut server) = bed.pair().await;
    let chunk = vec![7_u8; 1024 * 1024];
    // Until the queue of the proxy and each socket buffer are full: the proxy then holds
    // bytes that the client did not take.
    while matches!(timeout(QUIET, server.write_all(&chunk)).await, Ok(Ok(()))) {}
    in_time(bed.proxy.reset_all()).await;

    let mut buffer = vec![0_u8; 64 * 1024];
    let end = in_time(async {
        loop {
            match client.read(&mut buffer).await {
                Ok(1..) => {}
                end => return end,
            }
        }
    })
    .await;
    assert!(is_reset(&end), "the client reads the bytes that it has and then the reset: {end:?}");
}

/// With high load, the timer of `ResetEach` can end while the runtime does not run. The
/// test then changes the mode before the task of the proxy sees its timer. No reset must
/// follow the change.
#[tokio::test]
async fn reset_each_makes_no_reset_after_the_mode_change_when_its_timer_ended_first() {
    let period = Duration::from_millis(30);
    let bed = Bed::start().await;
    // A socket outside the proxy. Its bytes wake the test before the timer wakes the proxy.
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a free port");
    let wake = TcpStream::connect(listener.local_addr().expect("an address")).await.expect("the connect");
    let mut woken = listener.accept().await.expect("a connection").0;
    for round in 0..20 {
        let _open = bed.pair().await;
        let mut byte = [0_u8; 1];
        let mut read = pin!(woken.read(&mut byte));
        assert!(timeout(Duration::from_millis(1), &mut read).await.is_err(), "the read waits for the socket");
        bed.proxy.set(ProxyMode::ResetEach(period));
        // The task of the proxy runs: it reads the mode and starts its timer.
        yield_now().await;
        wake.try_write(b"x").expect("the kernel takes the byte");
        // The thread of the runtime is busy while the timer ends.
        std::thread::sleep(period * 4 / 3);
        in_time(read).await.expect("the byte arrives");

        let resets = bed.proxy.counters().reset;
        bed.proxy.set(ProxyMode::Pass);
        // The proxy accepts this connection after it read the mode and its timer.
        let _later = bed.pair().await;
        assert_eq!(bed.proxy.counters().reset, resets, "round {round}: a reset after the mode change");
    }
}
