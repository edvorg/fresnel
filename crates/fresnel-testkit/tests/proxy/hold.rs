//! The mode `HoldNew`: a part of the new connections gets no answer for a time.

use std::time::{Duration, Instant};

use fresnel_testkit::ProxyMode;
use tokio::io::AsyncWriteExt;
use tokio::time::timeout;

use super::{Bed, QUIET, in_time, is_quiet, receive};

const HOLD: Duration = Duration::from_millis(400);

#[tokio::test]
async fn hold_new_gives_a_new_connection_no_answer_for_its_time_and_then_passes() {
    let bed = Bed::start().await;
    let (mut open_client, mut open_server) = bed.pair().await;
    bed.proxy.set(ProxyMode::HoldNew { probability: 1.0, min: HOLD, max: HOLD });

    let started = Instant::now();
    let mut client = bed.connect().await;
    client.write_all(b"hello").await.expect("the kernel takes the bytes");
    open_client.write_all(b"open").await.expect("write");
    assert_eq!(receive(&mut open_server, 4).await, b"open", "an open connection works during the hold");

    let mut server = bed.accept().await;
    assert!(started.elapsed() >= HOLD, "the server saw the connection after {:?}", started.elapsed());
    assert_eq!(receive(&mut server, 5).await, b"hello");
    assert_eq!(bed.proxy.counters().held, 1);
}

#[tokio::test]
async fn hold_new_with_probability_zero_holds_no_connection() {
    let bed = Bed::start().await;
    let long = Duration::from_secs(600);
    bed.proxy.set(ProxyMode::HoldNew { probability: 0.0, min: long, max: long });
    for _ in 0..20 {
        drop(bed.pair().await);
    }
    assert_eq!(bed.proxy.counters().held, 0);
}

#[tokio::test]
async fn hold_new_holds_its_part_of_the_connections_and_the_same_ones_in_each_run() {
    let long = Duration::from_secs(600);
    let mut held = Vec::new();
    for _ in 0..2 {
        let bed = Bed::start().await;
        bed.proxy.set(ProxyMode::HoldNew { probability: 0.3, min: long, max: long });
        let mut clients = Vec::new();
        let mut which = Vec::new();
        for _ in 0..60 {
            clients.push(bed.connect().await);
            let before = bed.proxy.counters().held;
            let counters = in_time(bed.proxy.wait_for(|counters| counters.accepted == clients.len() as u64)).await;
            which.push(counters.held > before);
        }
        held.push(which);
    }
    let count = held[0].iter().filter(|held| **held).count();
    assert!((8..=28).contains(&count), "30 percent of 60 connections is 18, the proxy held {count}");
    assert_eq!(held[0], held[1], "the seed is fixed");
}

#[tokio::test]
async fn a_held_connection_gets_no_byte_from_the_server_side() {
    let bed = Bed::start().await;
    bed.proxy.set(ProxyMode::HoldNew {
        probability: 1.0,
        min: Duration::from_secs(600),
        max: Duration::from_secs(600),
    });
    let mut client = bed.connect().await;
    assert!(is_quiet(&mut client).await);
    assert!(timeout(QUIET, bed.upstream.accept()).await.is_err(), "no connect to the server");
}
