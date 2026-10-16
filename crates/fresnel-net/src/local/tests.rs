use std::net::{Ipv4Addr, Ipv6Addr, TcpListener as StdListener};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::*;

const LOOPBACK: [IpAddr; 2] = [IpAddr::V4(Ipv4Addr::LOCALHOST), IpAddr::V6(Ipv6Addr::LOCALHOST)];
const V4: [IpAddr; 1] = [IpAddr::V4(Ipv4Addr::LOCALHOST)];

/// What a client saw at the end of its connection.
#[derive(Debug, PartialEq, Eq)]
enum Seen {
    EndOfData,
    Reset,
    Nothing,
}

/// Reads to the end, and tells how the connection ended.
async fn read_to_end(client: &mut TcpStream, limit: Duration) -> (usize, Seen) {
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total = 0;
    loop {
        match timeout(limit, client.read(&mut buffer)).await {
            Ok(Ok(0)) => return (total, Seen::EndOfData),
            Ok(Ok(read)) => total += read,
            Ok(Err(_)) => return (total, Seen::Reset),
            Err(_) => return (total, Seen::Nothing),
        }
    }
}

#[tokio::test]
async fn an_automatic_port_is_the_same_on_each_address_and_each_one_takes_a_client() {
    let listener = LocalListener::bind(&LOOPBACK, 0).await.expect("a free port");
    let [first, second] = listener.addresses() else { panic!("two addresses: {:?}", listener.addresses()) };
    assert_eq!((first.ip(), second.ip()), (LOOPBACK[0], LOOPBACK[1]));
    assert!(first.port() != 0 && first.port() == second.port(), "{first} and {second}");
    for address in [*first, *second] {
        let mut client = TcpStream::connect(address).await.expect("a connect");
        let local = listener.accept().await.expect("an accept");
        assert_eq!(local.peer(), client.local_addr().expect("the address of the client"));
        local.finish().await;
        assert_eq!(read_to_end(&mut client, Duration::from_secs(5)).await, (0, Seen::EndOfData));
    }
}

#[tokio::test]
async fn a_port_of_an_other_listener_is_in_use_and_keeps_its_listener() {
    let holder = StdListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a listener");
    let address = holder.local_addr().expect("its address");
    let refused = LocalListener::bind(&V4, address.port()).await.expect_err("the port is in use");
    assert_eq!(refused, BindError::InUse(address));
    let text = format!("the local port {} is in use: an other program listens on {address}", address.port());
    assert_eq!(refused.to_string(), text);
    // The bind asked the holder with a connect of its own, which comes first.
    let client = std::net::TcpStream::connect(address).expect("a connect");
    let accepted = [(); 2].map(|()| holder.accept().expect("the holder keeps its port").0.peer_addr().ok());
    assert!(accepted.contains(&client.local_addr().ok()), "{accepted:?}");
}

/// The case that a default bind of tokio gets wrong on macOS: the app would take the
/// connections of the listener on each address.
#[tokio::test]
async fn a_wildcard_listener_of_an_other_program_makes_the_port_in_use() {
    let holder = StdListener::bind((Ipv4Addr::UNSPECIFIED, 0)).expect("a listener");
    let port = holder.local_addr().expect("its address").port();
    let refused = LocalListener::bind(&V4, port).await.expect_err("the port is in use");
    assert_eq!(refused, BindError::InUse(SocketAddr::new(V4[0], port)));
}

/// A forward that stops and starts again gets its fixed port back at once.
#[tokio::test]
async fn a_port_with_only_old_connections_in_time_wait_binds_again() {
    let listener = LocalListener::bind(&V4, 0).await.expect("a free port");
    let address = listener.addresses()[0];
    let mut client = TcpStream::connect(address).await.expect("a connect");
    // The app closes first, so its side of the connection goes to time-wait.
    listener.accept().await.expect("an accept").finish().await;
    assert_eq!(read_to_end(&mut client, Duration::from_secs(5)).await, (0, Seen::EndOfData));
    drop((client, listener));
    let again = LocalListener::bind(&V4, address.port()).await.expect("the port again");
    assert_eq!(again.addresses(), [address]);
}

#[tokio::test]
async fn an_address_of_no_interface_is_a_refusal_with_the_text_of_the_system() {
    let nowhere = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
    match LocalListener::bind(&[nowhere], 0).await {
        Err(BindError::Refused { address, text }) => assert!(address.ip() == nowhere && !text.is_empty()),
        other => panic!("a refusal: {other:?}"),
    }
}

/// Port forward prototype, test X5: a reset in the middle of a transfer was lost in 74 of
/// 100 rounds on macOS, and the client waited with no end. Each round here has bytes on the
/// way when the reset comes, and the client reads them first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reset_during_a_transfer_reaches_the_client_each_time() {
    let listener = LocalListener::bind(&V4, 0).await.expect("a free port");
    let address = listener.addresses()[0];
    for round in 0..40_usize {
        let mut client = TcpStream::connect(address).await.expect("a connect");
        let mut local = listener.accept().await.expect("an accept");
        let pieces = 20 + round % 30;
        let write = async {
            let chunk = vec![7_u8; 64 * 1024];
            let (_, mut writer) = local.split().expect("an open connection");
            for _ in 0..pieces {
                writer.write_all(&chunk).await.expect("the data");
            }
            local.reset().await;
        };
        let ((), (_, seen)) = tokio::join!(write, read_to_end(&mut client, Duration::from_secs(5)));
        assert_eq!(seen, Seen::Reset, "round {round}");
    }
}

/// A connection that drops with no close, at the stop of a forward or after a panic, gives
/// the client an error too, never a silent end of the data.
#[tokio::test]
async fn a_connection_that_drops_with_no_close_is_a_reset() {
    let listener = LocalListener::bind(&V4, 0).await.expect("a free port");
    let mut client = TcpStream::connect(listener.addresses()[0]).await.expect("a connect");
    // No byte waits in the connection of the app: a close with unread bytes is a reset on
    // each system, so the test would pass with no reset of the app.
    drop(listener.accept().await.expect("an accept"));
    assert_eq!(read_to_end(&mut client, Duration::from_secs(5)).await, (0, Seen::Reset));
}
