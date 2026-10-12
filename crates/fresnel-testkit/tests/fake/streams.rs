//! HTTP/2 streams: the watch stream, GOAWAY and the stream limit. Also the server behind
//! the fault proxy.

use std::pin::pin;

use fresnel_testkit::{FakeApiServer, FaultProxy};
use http::StatusCode;
use http_body_util::BodyExt;
use tokio::time::timeout;

use super::client::{get, in_time, next_line};
use super::{Bed, QUIET};

const PODS: &str = "/api/v1/pods";
const NODES: &str = "/api/v1/nodes";

#[tokio::test]
async fn a_watch_stream_gets_the_events_of_its_path_until_the_server_ends_it() {
    let bed = Bed::start().await;
    assert_eq!(bed.server.send_event(PODS, "{}"), 0, "no stream is open");
    let mut connection = bed.http2().await;
    let mut pods = connection.get("/api/v1/pods?watch=true&resourceVersion=1").await.into_body();
    let mut nodes = connection.get("/api/v1/nodes?allowWatchBookmarks=true&watch=1").await.into_body();
    let counters = bed.counters_when(|counters| counters.open_watches == 2).await;
    assert_eq!(counters.connections, 1, "the two streams use one connection");

    assert_eq!(bed.server.send_event(NODES, r#"{"type":"ADDED","object":{"kind":"Node"}}"#), 2);
    assert_eq!(bed.server.send_event(PODS, r#"{"type":"BOOKMARK","object":{"kind":"Pod"}}"#), 2);
    let pod_line = next_line(&mut pods).await;
    assert_eq!(pod_line.as_deref(), Some("{\"type\":\"BOOKMARK\",\"object\":{\"kind\":\"Pod\"}}\n"));
    assert_eq!(next_line(&mut nodes).await.as_deref(), Some("{\"type\":\"ADDED\",\"object\":{\"kind\":\"Node\"}}\n"));

    assert_eq!(bed.server.end_watches(), 2);
    assert_eq!((next_line(&mut pods).await, next_line(&mut nodes).await), (None, None), "a clean end of data");
    assert_eq!(bed.counters_when(|counters| counters.open_watches == 0).await.open_connections, 1);
}

#[tokio::test]
async fn a_watch_stream_that_the_client_drops_is_not_open() {
    let bed = Bed::start().await;
    let mut connection = bed.http2().await;
    let pods = connection.get("/api/v1/pods?watch=true").await;
    bed.counters_when(|counters| counters.open_watches == 1).await;
    drop(pods);
    bed.counters_when(|counters| counters.open_watches == 0).await;
}

#[tokio::test]
async fn go_away_lets_open_streams_live_and_moves_new_requests_to_a_new_connection() {
    let bed = Bed::start().await;
    let mut old = bed.http2().await;
    let mut pods = old.get("/api/v1/pods?watch=true").await.into_body();
    bed.counters_when(|counters| counters.open_watches == 1).await;

    bed.server.go_away();
    // hyper reads GOAWAY when it sends its next request. A request before that can pass.
    in_time(async { while old.try_get("/version").await.is_some() {} }).await;

    assert_eq!(bed.server.send_event(PODS, r#"{"type":"BOOKMARK"}"#), 1);
    assert_eq!(next_line(&mut pods).await.as_deref(), Some("{\"type\":\"BOOKMARK\"}\n"), "the open stream works");
    let mut new = bed.http2().await;
    assert_eq!(new.get("/version").await.status(), StatusCode::OK);
    assert_eq!(bed.server.counters().open_connections, 2);

    bed.server.end_watches();
    assert_eq!(next_line(&mut pods).await, None);
    let counters = bed.counters_when(|counters| counters.open_connections == 1).await;
    assert_eq!(counters.broken, 0, "the old connection closed after its last stream, with no error");
}

#[tokio::test]
async fn a_low_stream_limit_makes_the_next_request_wait_for_a_free_stream() {
    let bed = Bed::with(FakeApiServer::start_with_stream_limit(2).await.expect("a server"));
    let mut connection = bed.http2().await;
    let pods = connection.get("/api/v1/pods?watch=true").await;
    let _nodes = connection.get("/api/v1/nodes?watch=true").await;
    assert_eq!(connection.stream_limit(), 2);
    {
        let mut third = pin!(connection.send(get("/version")));
        assert!(timeout(QUIET, &mut third).await.is_err(), "the third request waits");
        assert_eq!(bed.server.counters().requests, 2, "the client did not send it");
        drop(pods);
        assert_eq!(in_time(third).await.expect("a response").status(), StatusCode::OK);
    }
    assert_eq!(bed.server.counters().requests, 3);
}

#[tokio::test]
async fn a_reset_of_a_fault_proxy_before_the_server_breaks_a_watch_stream() {
    let bed = Bed::start().await;
    let proxy = FaultProxy::start(bed.server.addr()).await.expect("a proxy");
    let mut connection = bed.trust.http2(proxy.addr()).await;
    let mut pods = connection.get("/api/v1/pods?watch=true").await.into_body();
    bed.counters_when(|counters| counters.open_watches == 1).await;

    in_time(proxy.reset_all()).await;
    assert!(matches!(in_time(pods.frame()).await, Some(Err(_))), "a reset is an error and not an end of data");
    let counters = bed.counters_when(|counters| counters.open_connections == 0).await;
    assert_eq!((counters.broken, counters.open_watches), (1, 0), "the server saw the reset");

    let mut new = bed.trust.http2(proxy.addr()).await;
    assert_eq!(new.get("/version").await.status(), StatusCode::OK);
    assert_eq!(proxy.task_failures(), Vec::<String>::new());
}
