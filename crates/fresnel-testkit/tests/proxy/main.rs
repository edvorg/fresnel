//! The fault proxy: one test for each mode. Each test holds the two ends of each connection,
//! a client socket and a socket of a listener behind the proxy.
//!
//! The proxy works on real sockets, so these tests use real time. A check that nothing
//! arrives waits for `QUIET`. Correct code cannot fail such a check on a slow machine.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod hold;
mod latency;
mod reset;
mod stall;

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use fresnel_testkit::{FaultProxy, ProxyCounters};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

/// The longest wait for a signal that must arrive.
const LIMIT: Duration = Duration::from_secs(20);
/// The time in which a fault must let nothing pass.
const QUIET: Duration = Duration::from_millis(200);

/// A proxy before a listener of the test.
struct Bed {
    proxy: FaultProxy,
    upstream: TcpListener,
}

impl Bed {
    async fn start() -> Self {
        let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a free port");
        let proxy = FaultProxy::start(upstream.local_addr().expect("an address")).await.expect("a proxy");
        Self { proxy, upstream }
    }

    async fn connect(&self) -> TcpStream {
        in_time(TcpStream::connect(self.proxy.addr())).await.expect("the proxy accepts")
    }

    async fn accept(&self) -> TcpStream {
        in_time(self.upstream.accept()).await.expect("a connection of the proxy").0
    }

    /// The two ends of one new connection: the client and the server.
    async fn pair(&self) -> (TcpStream, TcpStream) {
        let client = self.connect().await;
        (client, self.accept().await)
    }
}

impl Drop for Bed {
    /// A task of the proxy that ended is a fault of the test equipment in each test.
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert_eq!(self.proxy.task_failures(), Vec::<String>::new());
        }
    }
}

async fn in_time<T>(signal: impl Future<Output = T>) -> T {
    timeout(LIMIT, signal).await.expect("the signal arrives in time")
}

/// True when the socket gets no bytes, no end of data and no error in the quiet time.
async fn is_quiet(socket: &mut TcpStream) -> bool {
    timeout(QUIET, socket.read(&mut [0_u8; 16])).await.is_err()
}

async fn receive(socket: &mut TcpStream, count: usize) -> Vec<u8> {
    let mut bytes = vec![0_u8; count];
    in_time(socket.read_exact(&mut bytes)).await.expect("the bytes arrive");
    bytes
}

/// How the data of a socket ends: `Ok(0)` is the end of data, an error is a reset.
async fn end_of(socket: &mut TcpStream) -> io::Result<usize> {
    in_time(socket.read(&mut [0_u8; 16])).await
}

fn is_reset(end: &io::Result<usize>) -> bool {
    matches!(end, Err(error) if error.kind() == io::ErrorKind::ConnectionReset)
}

#[tokio::test]
async fn pass_carries_the_bytes_and_the_end_of_data_of_each_direction() {
    let bed = Bed::start().await;
    let (mut client, mut server) = bed.pair().await;
    client.write_all(b"request").await.expect("write");
    assert_eq!(receive(&mut server, 7).await, b"request");
    server.write_all(b"answer").await.expect("write");
    assert_eq!(receive(&mut client, 6).await, b"answer");

    client.shutdown().await.expect("shutdown");
    assert!(matches!(end_of(&mut server).await, Ok(0)), "the server sees the end of the client data");
    server.write_all(b"late").await.expect("the other direction is open");
    assert_eq!(receive(&mut client, 4).await, b"late");
    drop(server);
    assert!(matches!(end_of(&mut client).await, Ok(0)), "a clean close is not a reset");

    let counters = in_time(bed.proxy.wait_for(|counters| counters.open == 0)).await;
    assert_eq!(counters, ProxyCounters { accepted: 1, ..ProxyCounters::default() });
}

/// The proxy closes its sockets when the two directions ended. Bytes that a slow reader did
/// not take yet are then still in the send buffer of the proxy. A close with a reset drops them.
#[tokio::test]
async fn pass_gives_a_slow_reader_all_bytes_after_the_two_directions_ended() {
    const BYTES: usize = 8 * 1024 * 1024;
    let bed = Bed::start().await;
    let (mut client, mut server) = bed.pair().await;
    server.shutdown().await.expect("shutdown");
    assert!(matches!(end_of(&mut client).await, Ok(0)));

    let data = vec![5_u8; BYTES];
    let upload = async {
        client.write_all(&data).await.expect("write");
        client.shutdown().await.expect("shutdown");
    };
    let slow_download = async {
        let mut received = 0;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            match server.read(&mut buffer).await.expect("the bytes and then the end of data") {
                0 => return received,
                count => received += count,
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    };
    let ((), received) = in_time(async { tokio::join!(upload, slow_download) }).await;
    assert_eq!(received, BYTES);
    assert_eq!(in_time(bed.proxy.wait_for(|counters| counters.open == 0)).await.broken, 0);
}

#[tokio::test]
async fn a_reset_by_a_peer_goes_to_the_other_peer_and_shows_in_the_counters() {
    let bed = Bed::start().await;
    let (client, mut server) = bed.pair().await;
    socket2::SockRef::from(&client).set_linger(Some(Duration::ZERO)).expect("linger");
    drop(client);
    assert!(is_reset(&end_of(&mut server).await));
    let counters = in_time(bed.proxy.wait_for(|counters| counters.open == 0)).await;
    assert_eq!(counters, ProxyCounters { accepted: 1, broken: 1, ..ProxyCounters::default() });
}

#[tokio::test]
async fn a_server_that_is_down_gives_the_client_a_reset_and_shows_in_the_counters() {
    let closed: SocketAddr = {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a free port");
        listener.local_addr().expect("an address")
    };
    let proxy = FaultProxy::start(closed).await.expect("a proxy");
    let mut client = in_time(TcpStream::connect(proxy.addr())).await.expect("the proxy accepts");
    assert!(is_reset(&end_of(&mut client).await));
    let counters = in_time(proxy.wait_for(|counters| counters.open == 0)).await;
    assert_eq!(counters, ProxyCounters { accepted: 1, refused: 1, ..ProxyCounters::default() });
    assert_eq!(proxy.task_failures(), Vec::<String>::new());
}

/// A client that leaves at the moment of its connect is the fault of that one connection.
#[tokio::test]
async fn clients_that_reset_at_once_do_not_harm_an_open_connection() {
    let bed = Bed::start().await;
    let (mut client, mut server) = bed.pair().await;
    for _ in 0..100 {
        let leaving = bed.connect().await;
        socket2::SockRef::from(&leaving).set_linger(Some(Duration::ZERO)).expect("linger");
    }
    let counters = in_time(bed.proxy.wait_for(|counters| counters.accepted == 101 && counters.open == 1)).await;
    assert_eq!(counters.reset, 0, "the proxy reset no connection");
    client.write_all(b"alive").await.expect("write");
    assert_eq!(receive(&mut server, 5).await, b"alive");
}

#[tokio::test]
async fn the_drop_of_the_proxy_resets_its_connections_and_closes_its_port() {
    let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a free port");
    let proxy = FaultProxy::start(upstream.local_addr().expect("an address")).await.expect("a proxy");
    let addr = proxy.addr();
    let mut client = in_time(TcpStream::connect(addr)).await.expect("the proxy accepts");
    let mut server = in_time(upstream.accept()).await.expect("a connection of the proxy").0;

    drop(proxy);
    assert!(is_reset(&end_of(&mut client).await) && is_reset(&end_of(&mut server).await));
    assert!(in_time(TcpStream::connect(addr)).await.is_err(), "the port closed with the last task of the proxy");
}
