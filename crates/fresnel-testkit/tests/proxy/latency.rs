//! The mode `Latency`: each byte is late, and the data rate has no limit.

use std::time::{Duration, Instant};

use fresnel_testkit::ProxyMode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{Bed, in_time, receive};

const ONE_WAY: Duration = Duration::from_millis(100);

#[tokio::test]
async fn latency_delays_the_connect_by_one_round_trip_and_each_byte_by_one_way() {
    let bed = Bed::start().await;
    bed.proxy.set(ProxyMode::Latency(ONE_WAY));

    let started = Instant::now();
    let (mut client, mut server) = bed.pair().await;
    assert!(started.elapsed() >= 2 * ONE_WAY, "the connect needed {:?}", started.elapsed());

    let sent = Instant::now();
    client.write_all(b"ping").await.expect("write");
    assert_eq!(receive(&mut server, 4).await, b"ping");
    assert!(sent.elapsed() >= ONE_WAY, "the request needed {:?}", sent.elapsed());

    let sent = Instant::now();
    server.write_all(b"pong").await.expect("write");
    assert_eq!(receive(&mut client, 4).await, b"pong");
    assert!(sent.elapsed() >= ONE_WAY, "the answer needed {:?}", sent.elapsed());
}

/// A proxy that sleeps before each write passes one chunk of 64 KiB for each delay. It needs
/// 25 seconds or more for this transfer.
#[tokio::test]
async fn latency_does_not_limit_the_data_rate() {
    const BYTES: usize = 16 * 1024 * 1024;
    let bed = Bed::start().await;
    bed.proxy.set(ProxyMode::Latency(ONE_WAY));
    let (mut client, mut server) = bed.pair().await;

    let data = vec![3_u8; BYTES];
    let mut received = vec![0_u8; BYTES];
    let started = Instant::now();
    let (written, read) =
        in_time(async { tokio::join!(client.write_all(&data), server.read_exact(&mut received)) }).await;
    let took = started.elapsed();
    written.expect("write");
    read.expect("read");

    assert!(received == data, "the bytes arrive complete and in sequence");
    assert!(took >= ONE_WAY, "the transfer needed {took:?}");
    assert!(took < 30 * ONE_WAY, "the transfer needed {took:?}");
}
