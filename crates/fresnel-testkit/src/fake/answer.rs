//! The answer of the fake server to one request.

use std::convert::Infallible;
use std::future::pending;
use std::sync::Arc;

use bytes::Bytes;
use http::header::{CONNECTION, CONTENT_TYPE, RETRY_AFTER, UPGRADE};
use http::{HeaderValue, Request, Response, StatusCode, Version};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Empty, Full};

use super::script::FaultAnswer;
use super::seen::SeenRequest;
use super::state::Shared;
use super::watch;

pub(super) type Body = UnsyncBoxBody<Bytes, Infallible>;

/// The answer to `GET /version`. The app uses this path for its light request.
const VERSION: &str = r#"{"major":"1","minor":"33","gitVersion":"v1.33.5+fake","gitCommit":"0","gitTreeState":"clean","buildDate":"2026-01-01T00:00:00Z","goVersion":"go1.24","compiler":"gc","platform":"linux/arm64"}"#;
/// The answer to a path with no body of its own.
const EMPTY_LIST: &str = r#"{"kind":"List","apiVersion":"v1","metadata":{"resourceVersion":"1"},"items":[]}"#;
const UNAUTHORIZED: &str = r#"{"kind":"Status","apiVersion":"v1","metadata":{},"status":"Failure","message":"Unauthorized","reason":"Unauthorized","code":401}"#;
const TOO_MANY_REQUESTS: &str = r#"{"kind":"Status","apiVersion":"v1","metadata":{},"status":"Failure","message":"Too many requests, please try again later.","reason":"TooManyRequests","code":429}"#;

/// Records the request and gives its answer: the fault that takes it, or the normal answer.
/// The flag is true when the connection must end after this answer. A request with the
/// fault "no answer" never returns from here.
pub(super) async fn answer<B>(shared: &Arc<Shared>, connection: u64, request: Request<B>) -> (Response<Body>, bool) {
    let seen = SeenRequest::of(connection, &request);
    let fault = shared.fault_for(&seen);
    let ends = shared.ends_after(&seen, fault.as_ref());
    let response = match fault.map(|fault| fault.answer) {
        None => normal_answer(shared, &seen),
        Some(FaultAnswer::Unauthorized) => json(StatusCode::UNAUTHORIZED, UNAUTHORIZED.into()),
        Some(FaultAnswer::TooManyRequests { retry_after_seconds }) => {
            let mut response = json(StatusCode::TOO_MANY_REQUESTS, TOO_MANY_REQUESTS.into());
            response.headers_mut().insert(RETRY_AFTER, HeaderValue::from(retry_after_seconds));
            response
        }
        Some(FaultAnswer::Nothing) => {
            shared.record(seen);
            shared.count(|counters| counters.unanswered += 1);
            return pending().await;
        }
    };
    shared.record(seen);
    (response, ends)
}

fn normal_answer(shared: &Arc<Shared>, request: &SeenRequest) -> Response<Body> {
    let path = request.uri.path();
    match request.headers.get(UPGRADE) {
        Some(protocol) if request.version == Version::HTTP_11 => switching_protocols(protocol.clone()),
        _ if request.is_watch() => json_body(StatusCode::OK, watch::body(shared, path)),
        _ if path == "/version" => json(StatusCode::OK, VERSION.into()),
        _ => json(StatusCode::OK, shared.body(path).unwrap_or_else(|| EMPTY_LIST.into())),
    }
}

/// The answer that changes an HTTP/1.1 connection to a raw stream.
fn switching_protocols(protocol: HeaderValue) -> Response<Body> {
    let mut response = Response::new(Empty::new().boxed_unsync());
    *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    response.headers_mut().insert(CONNECTION, HeaderValue::from_static("Upgrade"));
    response.headers_mut().insert(UPGRADE, protocol);
    response
}

fn json(status: StatusCode, text: Bytes) -> Response<Body> {
    json_body(status, Full::new(text).boxed_unsync())
}

fn json_body(status: StatusCode, body: Body) -> Response<Body> {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}
