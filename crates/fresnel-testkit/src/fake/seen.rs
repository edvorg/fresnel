//! The requests that the fake server saw, for the checks of a test.

use http::header::AUTHORIZATION;
use http::{HeaderMap, Method, Request, Uri, Version};

/// The head of one request.
#[derive(Clone, Debug)]
pub struct SeenRequest {
    /// The number of the connection in the sequence of accepts, from 0.
    pub connection: u64,
    /// The method of the request.
    pub method: Method,
    /// The path and the query of the request.
    pub uri: Uri,
    /// HTTP/2 or HTTP/1.1.
    pub version: Version,
    /// All headers of the request.
    pub headers: HeaderMap,
}

impl SeenRequest {
    pub(super) fn of<B>(connection: u64, request: &Request<B>) -> Self {
        Self {
            connection,
            method: request.method().clone(),
            uri: request.uri().clone(),
            version: request.version(),
            headers: request.headers().clone(),
        }
    }

    /// The bearer token of the request, if it has one.
    pub fn bearer(&self) -> Option<&str> {
        self.headers.get(AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ")
    }

    /// True for a watch request or a log that follows: its answer is a stream of the lines
    /// that the test sends.
    pub fn is_watch(&self) -> bool {
        let query = self.uri.query().unwrap_or_default();
        query.split('&').any(|pair| matches!(pair, "watch=true" | "watch=1" | "follow=true"))
    }
}
