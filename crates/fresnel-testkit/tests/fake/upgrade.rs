//! The HTTP 101 upgrade of an HTTP/1.1 connection, as exec and port forward use it.

use bytes::Bytes;
use fresnel_testkit::Fault;
use http::header::{CONNECTION, UPGRADE};
use http::{Request, StatusCode, Version};
use http_body_util::Empty;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::Bed;
use super::client::in_time;

const EXEC: &str = "/api/v1/namespaces/default/pods/shell/exec?command=sh";

fn upgrade_request() -> Request<Empty<Bytes>> {
    let request = Request::get(EXEC).header("host", "kit").header(CONNECTION, "Upgrade").header(UPGRADE, "websocket");
    request.body(Empty::new()).expect("a request")
}

#[tokio::test]
async fn an_upgrade_request_gets_http_101_and_a_raw_stream_that_sends_each_byte_back() {
    let bed = Bed::start().await;
    let mut connection = bed.http1().await;
    let (head, _) = connection.fetch(super::get("/version")).await;
    assert_eq!(head.status, StatusCode::OK, "a request before the upgrade, as the test of a warm connection");

    let mut stream = connection.upgrade(upgrade_request()).await;
    assert_eq!(bed.counters_when(|counters| counters.upgrades == 1).await.connections, 1);
    for message in [&b"first"[..], &b"second message"[..]] {
        stream.write_all(message).await.expect("write");
        let mut back = vec![0_u8; message.len()];
        in_time(stream.read_exact(&mut back)).await.expect("the bytes come back");
        assert_eq!(back, message);
    }

    stream.shutdown().await.expect("shutdown");
    assert_eq!(in_time(stream.read(&mut [0_u8; 8])).await.expect("the end of data"), 0);
    let counters = bed.counters_when(|counters| counters.open_connections == 0).await;
    assert_eq!(counters.broken, 0);
    let seen = bed.server.requests();
    assert_eq!((seen[1].version, seen[1].uri.path_and_query().map(|uri| uri.as_str())), (Version::HTTP_11, Some(EXEC)));
}

#[tokio::test]
async fn a_fault_takes_an_upgrade_request_before_the_upgrade() {
    let bed = Bed::start().await;
    bed.server.add_fault(Fault::unauthorized().on_path("/api/v1/namespaces/default/pods/shell/exec"));
    let mut connection = bed.http1().await;
    let (head, _) = connection.fetch(upgrade_request()).await;
    assert_eq!(head.status, StatusCode::UNAUTHORIZED);
    assert_eq!(bed.server.counters().upgrades, 0);
}
