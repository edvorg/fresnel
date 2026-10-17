//! A forward with no cluster: the listener, the wait of a local connection and the stop.
//! The tests of the tunnels run on the fake API server (`tests/sockets/forwards`).

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener as StdListener};

use fresnel_net::task::TaskReport;
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::time::{Instant, timeout};

use super::*;
use crate::cluster::ClusterId;
use crate::kinds::Kind;
use crate::published::{Changes, Disposal};
use crate::store::ObjectKey;

const LIMIT: Duration = Duration::from_secs(10);

/// The tasks, the bin and the notice of one test.
struct Bed {
    parts: ForwardParts,
    changes: Changes,
    _disposal: Disposal,
}

impl Bed {
    fn new(wait: Duration) -> Self {
        Self::with(ForwardSettings { wait, ..ForwardSettings::default() })
    }

    fn with(settings: ForwardSettings) -> Self {
        let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task ended: {report}"));
        let disposal = Disposal::start(&tasks);
        let notice = Notice::new();
        let journal = Journal::new(100, WallClock::system());
        let parts = ForwardParts { tasks, journal, clock: WallClock::system(), bin: disposal.bin(), notice, settings };
        Self { changes: parts.notice.changes(), parts, _disposal: disposal }
    }

    fn start(&self, local: LocalPort) -> Forward {
        Forward::start(rule(local), Err("ams connects".to_owned()), self.parts.clone())
    }

    /// Waits until the view of the forward meets the condition.
    async fn until(&mut self, forward: &Forward, what: &str, met: impl Fn(&ForwardView) -> bool) -> ForwardView {
        let wait = async {
            loop {
                let view = forward.view();
                if met(&view) {
                    return ForwardView::clone(&view);
                }
                self.changes.next().await;
            }
        };
        timeout(LIMIT, wait).await.unwrap_or_else(|_| panic!("in time: {what}: {:?}", *forward.view()))
    }
}

fn rule(local: LocalPort) -> ForwardRule {
    ForwardRule {
        cluster: ClusterId::new("ams"),
        kind: Kind::Pod,
        key: ObjectKey::new("shop", "web-1"),
        port: 8080,
        local,
        bind: Bind::Address(IpAddr::V4(Ipv4Addr::LOCALHOST)),
    }
}

/// How a local client saw the end of its connection, and when.
async fn end_of(mut client: TcpStream) -> (&'static str, Instant) {
    let mut buffer = [0_u8; 64];
    let seen = match timeout(LIMIT, client.read(&mut buffer)).await {
        Ok(Ok(0)) => "end of data",
        Ok(Ok(_)) => "data",
        Ok(Err(_)) => "reset",
        Err(_) => "nothing",
    };
    (seen, Instant::now())
}

/// Plan section 15, M5: a local port in use gives the state "failed" with that reason, and
/// the program that holds the port keeps it.
#[tokio::test]
async fn a_fixed_port_in_use_fails_with_its_reason_and_never_moves() {
    let mut bed = Bed::new(Duration::from_secs(5));
    let holder = StdListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a listener");
    let address = holder.local_addr().expect("its address");
    let forward = bed.start(LocalPort::Fixed(address.port()));
    let failed = bed.until(&forward, "the state failed", |view| view.state.name() == "failed").await;
    let text = format!("failed: the local port {} is in use: an other program listens on {address}", address.port());
    assert_eq!(failed.state.to_string(), text);
    assert_eq!(failed.addresses, Vec::<SocketAddr>::new(), "no other port");
    assert!(std::net::TcpStream::connect(address).is_ok(), "the holder keeps its port");
}

/// A local connection that arrives before a target waits for the wait time, then gets a
/// reset, and the listener stays open for the next one.
#[tokio::test]
async fn a_local_connection_with_no_target_waits_and_gets_a_reset_and_the_port_stays_open() {
    let wait = Duration::from_millis(400);
    let mut bed = Bed::new(wait);
    let forward = bed.start(LocalPort::Automatic);
    let starting = bed.until(&forward, "a local address", |view| view.address().is_some()).await;
    assert_eq!(starting.state, ForwardState::Starting("ams connects".to_owned()));
    let address = starting.address().expect("an address");
    for round in 0..2 {
        let connected = Instant::now();
        let client = TcpStream::connect(address).await.expect("the port is open");
        let (seen, at) = end_of(client).await;
        assert_eq!(seen, "reset", "round {round}");
        assert!(at - connected >= wait, "the reset came after {:?}", at - connected);
    }
    let view = bed.until(&forward, "the counts", |view| view.connections.reset == 2).await;
    assert_eq!((view.connections.total, view.connections.open), (2, 0));
    let error = view.last_error.map(|(_, text)| text);
    assert_eq!(error.as_deref(), Some("a local connection got no tunnel in 0.4 s: a reset"));
}

/// A stop closes the listener and resets each local connection; the last view stays.
#[tokio::test]
async fn a_stop_closes_the_port_and_resets_each_local_connection() {
    let mut bed = Bed::new(Duration::from_secs(30));
    let forward = bed.start(LocalPort::Automatic);
    let view = bed.until(&forward, "a local address", |view| view.address().is_some()).await;
    let address = view.address().expect("an address");
    let client = TcpStream::connect(address).await.expect("the port is open");
    bed.until(&forward, "an open connection", |view| view.connections.open == 1).await;
    let stopped = forward.stop();
    assert_eq!((&stopped.state, stopped.connections.open, stopped.connections.total), (&ForwardState::Stopped, 0, 1));
    assert_eq!(end_of(client).await.0, "reset");
    // The stop of the task ends its listener a moment later.
    let closed = async {
        while TcpStream::connect(address).await.is_ok() {
            tokio::task::yield_now().await;
        }
    };
    timeout(LIMIT, closed).await.expect("the port closes");
}

/// Each local connection holds a tunnel and an upgrade connection to the API server, so a
/// forward has a limit of open ones. A connection above it gets a reset at once, not after the
/// wait, and the journal gets one entry for the connections above it in a minute.
#[tokio::test]
async fn a_local_connection_above_the_limit_gets_a_reset_at_once_and_one_entry() {
    let wait = Duration::from_secs(30);
    let mut bed = Bed::with(ForwardSettings { wait, connections: 2, ..ForwardSettings::default() });
    let forward = bed.start(LocalPort::Automatic);
    let view = bed.until(&forward, "a local address", |view| view.address().is_some()).await;
    let address = view.address().expect("an address");
    let mut waiting = Vec::new();
    for _ in 0..2 {
        waiting.push(TcpStream::connect(address).await.expect("the port is open"));
    }
    bed.until(&forward, "two open connections", |view| view.connections.open == 2).await;
    for _ in 0..2 {
        let connected = Instant::now();
        let (seen, at) = end_of(TcpStream::connect(address).await.expect("the port is open")).await;
        assert_eq!(seen, "reset", "above the limit");
        assert!(at - connected < wait, "the reset came at once, not after the wait");
    }
    let view = bed.until(&forward, "the counts", |view| view.connections.reset == 2).await;
    assert_eq!((view.connections.open, view.connections.total), (2, 4));
    let rule = rule(LocalPort::Automatic);
    let text = format!("{rule}: a new local connection got a reset: 2 are open, the limit of the forward");
    let entries = bed.parts.journal.view().entries.iter().map(|entry| entry.text.clone()).collect::<Vec<_>>();
    assert_eq!(entries.iter().filter(|entry| entry.contains("the limit")).collect::<Vec<_>>(), [&text]);
    drop(waiting);
}
