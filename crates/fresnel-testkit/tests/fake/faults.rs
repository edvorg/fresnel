//! The scripted faults: no answer, HTTP 401 by path, by token and by time, HTTP 403 and HTTP 429.

use std::pin::pin;
use std::time::Duration;

use fresnel_testkit::Fault;
use http::StatusCode;
use http::header::AUTHORIZATION;
use tokio::time::{sleep, timeout};

use super::client::{get, in_time, text};
use super::{Bed, QUIET};

#[tokio::test]
async fn no_answer_holds_the_request_and_the_connection_serves_other_paths() {
    let bed = Bed::start().await;
    bed.server.add_fault(Fault::no_answer().on_path("/api/v1/pods"));
    let mut connection = bed.http2().await;
    {
        let mut held = pin!(connection.send(get("/api/v1/pods?limit=500")));
        assert!(timeout(QUIET, &mut held).await.is_err(), "the request gets no answer");
    }
    assert_eq!(connection.get("/version").await.status(), StatusCode::OK);
    let counters = bed.server.counters();
    assert_eq!((counters.requests, counters.unanswered, counters.connections), (2, 1, 1));
}

#[tokio::test]
async fn unauthorized_by_path_refuses_one_path_and_no_other() {
    let bed = Bed::start().await;
    bed.server.add_fault(Fault::unauthorized().on_path("/apis/metrics.k8s.io/"));
    let mut connection = bed.http2().await;
    let refused = connection.get("/apis/metrics.k8s.io/v1beta1/nodes").await;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert!(text(refused).await.contains(r#""reason":"Unauthorized","code":401"#));
    assert_eq!(connection.get("/api/v1/nodes").await.status(), StatusCode::OK);
    assert_eq!(connection.get("/apis/metrics.k8s.io").await.status(), StatusCode::OK, "the prefix is exact");
}

#[tokio::test]
async fn unauthorized_by_token_refuses_the_old_token_and_takes_a_new_one() {
    let bed = Bed::start().await;
    bed.server.add_fault(Fault::unauthorized().for_token("old"));
    let mut connection = bed.http2().await;
    let mut status = Vec::new();
    for token in ["Bearer old", "Bearer new"] {
        let mut request = get("/api/v1/pods");
        request.headers_mut().insert(AUTHORIZATION, token.parse().expect("a header"));
        status.push(in_time(connection.send(request)).await.expect("a response").status());
    }
    status.push(connection.get("/api/v1/pods").await.status());
    assert_eq!(status, [StatusCode::UNAUTHORIZED, StatusCode::OK, StatusCode::OK]);
}

#[tokio::test]
async fn unauthorized_by_time_ends_by_itself() {
    let outage = Duration::from_secs(1);
    let bed = Bed::start().await;
    let mut connection = bed.http2().await;
    bed.server.add_fault(Fault::unauthorized().for_time(outage));
    assert_eq!(connection.get("/api/v1/pods").await.status(), StatusCode::UNAUTHORIZED);
    sleep(outage).await;
    assert_eq!(connection.get("/api/v1/pods").await.status(), StatusCode::OK);
}

#[tokio::test]
async fn forbidden_gives_http_403_with_the_status_object_of_a_server() {
    let bed = Bed::start().await;
    bed.server.add_fault(Fault::forbidden().on_path("/api/v1/secrets"));
    let mut connection = bed.http2().await;
    let refused = connection.get("/api/v1/secrets?limit=500").await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert!(text(refused).await.contains(r#""reason":"Forbidden","code":403"#));
    assert_eq!(connection.get("/api/v1/pods").await.status(), StatusCode::OK);
}

#[tokio::test]
async fn too_many_requests_gives_http_429_with_retry_after() {
    let bed = Bed::start().await;
    bed.server.add_fault(Fault::too_many_requests(Duration::from_secs(7)));
    let mut connection = bed.http2().await;
    let throttled = connection.get("/api/v1/pods").await;
    assert_eq!(throttled.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(throttled.headers().get("retry-after").expect("the header"), "7");
    assert!(text(throttled).await.contains(r#""reason":"TooManyRequests","code":429"#));
}

#[tokio::test]
async fn the_first_fault_that_takes_a_request_wins_and_a_clear_removes_all() {
    let bed = Bed::start().await;
    bed.server.add_fault(Fault::too_many_requests(Duration::from_secs(1)).on_path("/api/v1/pods"));
    bed.server.add_fault(Fault::unauthorized());
    let mut connection = bed.http2().await;
    assert_eq!(connection.get("/api/v1/pods").await.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(connection.get("/api/v1/nodes").await.status(), StatusCode::UNAUTHORIZED);
    bed.server.clear_faults();
    assert_eq!(connection.get("/api/v1/pods").await.status(), StatusCode::OK);
}
