//! The fake API server: its protocols, its answers and each scripted fault.
//!
//! The server works on real sockets, so these tests use real time. A check that no answer
//! arrives waits for `QUIET`. Correct code cannot fail such a check on a slow machine.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "../common/client.rs"]
mod client;
mod faults;
mod streams;
mod upgrade;

use std::time::Duration;

use fresnel_testkit::{FakeApiServer, FakeCounters};
use http::header::AUTHORIZATION;
use http::{StatusCode, Version};
use rustls::pki_types::CertificateDer;

use client::{Http1, Http2, STREAMS_BEFORE_SETTINGS, Trust, get, in_time, text};

/// The time in which a fault must give no answer.
const QUIET: Duration = Duration::from_millis(200);

/// A fake server and the TLS settings of a client that trusts it.
struct Bed {
    server: FakeApiServer,
    trust: Trust,
}

impl Bed {
    async fn start() -> Self {
        Self::with(FakeApiServer::start().await.expect("a server"))
    }

    fn with(server: FakeApiServer) -> Self {
        let certificate = CertificateDer::from(server.certificate_der().to_vec());
        Self { server, trust: Trust { roots: vec![certificate], identity: None } }
    }

    async fn http2(&self) -> Http2 {
        self.trust.http2(self.server.addr()).await
    }

    async fn http1(&self) -> Http1 {
        self.trust.http1(self.server.addr()).await
    }

    async fn counters_when(&self, condition: impl FnMut(&FakeCounters) -> bool) -> FakeCounters {
        in_time(self.server.wait_for(condition)).await
    }
}

impl Drop for Bed {
    /// A task of the server that ended is a fault of the test equipment in each test.
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert_eq!(self.server.task_failures(), Vec::<String>::new());
        }
    }
}

#[tokio::test]
async fn the_server_speaks_http2_over_tls_and_keeps_the_head_of_each_request() {
    let bed = Bed::start().await;
    let mut connection = bed.http2().await;

    let mut request = get("/version?timeout=5s");
    request.headers_mut().insert(AUTHORIZATION, "Bearer token-1".parse().expect("a header"));
    let response = in_time(connection.send(request)).await.expect("a response");
    assert_eq!((response.status(), response.version()), (StatusCode::OK, Version::HTTP_2));
    assert!(text(response).await.contains(r#""gitVersion":"v1.33.5+fake""#));
    assert_ne!(STREAMS_BEFORE_SETTINGS, 100, "the client starts with an other limit");
    assert_eq!(connection.stream_limit(), 100, "the server gave the stream limit of a kube-apiserver");

    let seen = bed.server.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!((seen[0].connection, seen[0].version, seen[0].method.as_str()), (0, Version::HTTP_2, "GET"));
    assert_eq!(
        (seen[0].uri.path(), seen[0].uri.query(), seen[0].bearer()),
        ("/version", Some("timeout=5s"), Some("token-1"))
    );
    let counters = bed.server.counters();
    assert_eq!((counters.connections, counters.open_connections, counters.requests), (1, 1, 1));
}

#[tokio::test]
async fn the_server_speaks_http1_with_a_client_that_offers_only_that() {
    let bed = Bed::start().await;
    let mut connection = bed.http1().await;
    let (head, body) = connection.fetch(get("/version")).await;
    assert_eq!((head.status, head.version), (StatusCode::OK, Version::HTTP_11));
    assert!(body.starts_with(b"{\"major\":\"1\""));
    let (head, _) = connection.fetch(get("/version")).await;
    assert_eq!(head.status, StatusCode::OK, "the connection stays open for the next request");

    let seen = bed.server.requests();
    assert_eq!(
        seen.iter().map(|request| (request.connection, request.version)).collect::<Vec<_>>(),
        [(0, Version::HTTP_11); 2]
    );
    assert_eq!(seen[0].bearer(), None);
}

#[tokio::test]
async fn a_path_gets_the_body_of_the_test_and_each_other_path_an_empty_list() {
    let bed = Bed::start().await;
    bed.server.set_body("/api/v1/pods", r#"{"kind":"PodList","items":[{"metadata":{"name":"a"}}]}"#);
    let mut connection = bed.http2().await;
    let pods = text(connection.get("/api/v1/pods?limit=500").await).await;
    assert_eq!(pods, r#"{"kind":"PodList","items":[{"metadata":{"name":"a"}}]}"#);
    let services = connection.get("/api/v1/services").await;
    assert_eq!(services.headers().get("content-type").expect("a type"), "application/json");
    assert_eq!(
        text(services).await,
        r#"{"kind":"List","apiVersion":"v1","metadata":{"resourceVersion":"1"},"items":[]}"#
    );
}

#[tokio::test]
async fn a_client_that_leaves_in_the_handshake_is_a_count_and_not_a_fault_of_the_server() {
    let bed = Bed::start().await;
    drop(in_time(tokio::net::TcpStream::connect(bed.server.addr())).await.expect("the TCP connect"));
    let counters = bed.counters_when(|counters| counters.broken == 1 && counters.open_connections == 0).await;
    assert_eq!((counters.connections, counters.requests), (1, 0));
    assert_eq!(bed.http2().await.get("/version").await.status(), StatusCode::OK, "the server still works");
}
