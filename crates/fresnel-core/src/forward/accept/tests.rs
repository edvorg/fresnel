//! The pause after a failed accept and its entries in the journal, with injected errors and
//! paused time. The limit of the open connections has its test on a forward (`forward/tests`).

use std::net::{IpAddr, Ipv4Addr, TcpStream};

use fresnel_net::WallClock;

use super::*;
use crate::cluster::ClusterId;
use crate::forward::{Bind, Connections, LocalPort};
use crate::kinds::Kind;
use crate::store::ObjectKey;

fn rule() -> ForwardRule {
    ForwardRule {
        cluster: ClusterId::new("ams"),
        kind: Kind::Pod,
        key: ObjectKey::new("shop", "web-1"),
        port: 8080,
        local: LocalPort::Automatic,
        bind: Bind::Address(IpAddr::V4(Ipv4Addr::LOCALHOST)),
    }
}

/// The error of an accept with each descriptor of the process in use. The text of the system
/// differs on each one.
fn too_many_files() -> io::Error {
    io::Error::other("too many open files")
}

fn entries(journal: &Journal) -> Vec<String> {
    journal.view().entries.iter().map(|entry| format!("{:?} {}: {}", entry.level, entry.source, entry.text)).collect()
}

/// The pause of `accepts` before the accept of a client that waits in the queue of the listener.
async fn pause(accepts: &Accepts, listener: &LocalListener, clients: &mut Vec<TcpStream>) -> (Duration, LocalStream) {
    clients.push(TcpStream::connect(listener.addresses()[0]).expect("a connect"));
    let started = Instant::now();
    let local = accepts.next(listener).await.expect("the client");
    (started.elapsed(), local)
}

/// An accept that fails fails again at once. Before, the journal got an entry each 100 ms:
/// 10,000 entries in 17 minutes. Now each failure in a row doubles the pause before the next
/// accept, to 2 s, and an accept that works ends it. The journal gets the first failure at
/// once and the later ones in one entry a minute after it, with their count.
#[tokio::test(start_paused = true)]
async fn a_failed_accept_pauses_longer_each_time_and_the_journal_gets_one_entry_a_minute() {
    let listener = LocalListener::bind(&[IpAddr::V4(Ipv4Addr::LOCALHOST)], 0).await.expect("a listener");
    let (rule, journal, counts) = (rule(), Journal::new(100, WallClock::system()), Counts::default());
    let (mut accepts, mut clients, mut pauses) = (Accepts::new(8), Vec::new(), Vec::new());
    for _ in 0..7 {
        assert!(accepts.take(Err(too_many_files()), 0, &counts).is_none(), "no connection");
        accepts.note(&rule, &journal);
        pauses.push(pause(&accepts, &listener, &mut clients).await.0);
    }
    let ms = Duration::from_millis;
    assert_eq!(pauses, [ms(100), ms(200), ms(400), ms(800), ms(1600), ms(2000), ms(2000)]);
    let text = "an accept of a local connection failed: too many open files; the next one waits up to 2 s";
    let first = format!("Warning ams: {rule}: {text}");
    assert_eq!(entries(&journal), std::slice::from_ref(&first), "one entry in a minute");

    let (_, local) = pause(&accepts, &listener, &mut clients).await;
    assert!(accepts.take(Ok(local), 0, &counts).is_some(), "an accept that works");
    assert!(accepts.take(Err(too_many_files()), 0, &counts).is_none());
    assert_eq!(pause(&accepts, &listener, &mut clients).await.0, ms(100), "the first pause again");
    tokio::time::sleep(ENTRY_GAP).await;
    accepts.note(&rule, &journal);
    let text = "7 accepts of a local connection failed since the last entry, the last: too many open files";
    assert_eq!(entries(&journal), [first, format!("Warning ams: {rule}: {text}")]);
    assert_eq!(counts.read().0, Connections::default(), "a failed accept is no connection");
}
