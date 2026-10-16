//! The gateway of one cluster: the requests of the app to its API server.
//!
//! This is the only module that holds a kube client (plan section 7.2). The client sends
//! each request through the connection layer of the cluster. The gateway has six functions,
//! one for each kind of request. The kind selects the connection and the time limits of the
//! connection layer (plan section 8.10):
//!
//! - `get`: 10 s for the full answer.
//! - `list_page`: 15 s with no data, 120 s in total.
//! - `watch_stream` and `log_stream`: 10 s for the start. The stream has no limit.
//! - `send`: 15 s for the full answer. A write goes out one time (design rule 6).
//! - `upgrade`: 10 s for the answer of the server. The WebSocket then has its own ping.
//!
//! A caller builds its request with `kube::core::Request`, with a path and no host.
//!
//! - `bridge`: the service between the kube client and the connection layer.
//! - `answer`: the status and the body of an answer. `stream`: watch events and log data.
//! - `write`: what the app knows about a write that failed.

mod answer;
mod bridge;
mod stream;
mod write;

use fresnel_net::{ErrorClass, Limits, Offer, RequestKind, Socket, SocketSpec};
use http::{Method, Request, Response, StatusCode};
use kube::client::Body;
use serde::de::DeserializeOwned;
use tokio::runtime::Handle;

pub use kube::core::WatchEvent;
pub use stream::{LogStream, WatchStream};
pub use write::{Fate, WriteError};

use super::ClusterId;
use crate::error::CoreError;

/// The requests to one cluster. A clone is one more handle to the same connections.
#[derive(Clone)]
pub struct Gateway {
    cluster: ClusterId,
    client: kube::Client,
    /// The connection layer, which turns the answer to an upgrade into a socket.
    net: fresnel_net::Cluster,
    limits: Limits,
}

impl Gateway {
    /// `runtime` runs the one task that the kube client starts; see `bridge`.
    pub(super) fn new(cluster: ClusterId, net: fresnel_net::Cluster, limits: Limits, runtime: &Handle) -> Self {
        let _context = runtime.enter();
        // The gateway gives a full path to each request, so the namespace is not in use.
        let client = kube::Client::new(bridge::Bridge::new(net.clone()), "default");
        Self { cluster, client, net, limits }
    }

    /// The cluster of this gateway.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// Reads one object or one other small answer.
    pub async fn get<T: DeserializeOwned>(&self, request: Request<Vec<u8>>) -> Result<T, CoreError> {
        answer::json(self.read(RequestKind::Get, "get", request).await?).await
    }

    /// Reads one page of a list.
    pub async fn list_page<T: DeserializeOwned>(&self, request: Request<Vec<u8>>) -> Result<T, CoreError> {
        answer::json(self.read(RequestKind::ListPage, "list_page", request).await?).await
    }

    /// Starts a watch. The result comes after the good status of the server, so a caller
    /// can show the watch as live.
    pub async fn watch_stream<K: DeserializeOwned>(
        &self,
        request: Request<Vec<u8>>,
    ) -> Result<WatchStream<K>, CoreError> {
        let response = self.read(RequestKind::Watch, "watch_stream", request).await?;
        Ok(WatchStream::new(response.into_body()))
    }

    /// Starts a log stream.
    pub async fn log_stream(&self, request: Request<Vec<u8>>) -> Result<LogStream, CoreError> {
        let response = self.read(RequestKind::LogStream, "log_stream", request).await?;
        Ok(LogStream::new(response.into_body()))
    }

    /// Sends one write: create, replace, patch or delete. The gateway never sends it a
    /// second time. The error tells what the caller can do next.
    pub async fn send<T: DeserializeOwned>(&self, request: Request<Vec<u8>>) -> Result<T, WriteError> {
        if is_read(request.method()) {
            return Err(WriteError::not_sent(wrong_method("send", "a write", &request)));
        }
        let response = self.exchange(RequestKind::Write, request).await.map_err(WriteError::of)?;
        answer::json(response).await.map_err(WriteError::after_acceptance)
    }

    /// Changes a warm or a new connection to a WebSocket of one of the subprotocols of `spec`:
    /// exec, attach or port forward. The socket has the ping of the connection layer.
    ///
    /// The gateway sends the handshake itself, not with the WebSocket call of kube, which
    /// drops the body of a refusal: the message of the server, for example of a container
    /// that does not exist, is the text of the error.
    pub async fn upgrade(&self, mut request: Request<Vec<u8>>, spec: SocketSpec) -> Result<Socket, CoreError> {
        if !is_read(request.method()) {
            return Err(wrong_method("upgrade", "an upgrade", &request));
        }
        let offer = Offer::new(spec.protocols);
        offer.apply(request.headers_mut())?;
        let response = self.exchange(RequestKind::Upgrade, request).await?;
        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            let code = response.status().as_u16();
            let text = format!("the server answered the upgrade with HTTP {code}, not with a WebSocket");
            return Err(CoreError::new(ErrorClass::Server, text));
        }
        Ok(self.net.open_socket(response, &offer, spec).await?)
    }

    /// Sends a read of one kind. The answer comes back after the check of its status.
    async fn read(
        &self,
        kind: RequestKind,
        function: &str,
        request: Request<Vec<u8>>,
    ) -> Result<Response<Body>, CoreError> {
        if !is_read(request.method()) {
            return Err(wrong_method(function, "a read", &request));
        }
        self.exchange(kind, request).await
    }

    /// Sends one request with its kind. An answer with an error status is an error. HTTP 101
    /// is the success of an upgrade.
    async fn exchange(&self, kind: RequestKind, mut request: Request<Vec<u8>>) -> Result<Response<Body>, CoreError> {
        request.extensions_mut().insert(kind);
        let response = self.client.send(request.map(Body::from)).await.map_err(answer::transport)?;
        if response.status().is_success() || response.status() == StatusCode::SWITCHING_PROTOCOLS {
            return Ok(response);
        }
        Err(answer::refusal(response, self.limits.get).await)
    }
}

impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gateway").field("cluster", &self.cluster).finish_non_exhaustive()
    }
}

fn is_read(method: &Method) -> bool {
    method == Method::GET
}

/// A request in the incorrect function would get the incorrect connection and time limits,
/// and a write in a read function would miss the rule for writes.
fn wrong_method(function: &str, takes: &str, request: &Request<Vec<u8>>) -> CoreError {
    CoreError::invalid(format!("`{function}` sends {takes}, and the request has the method {}", request.method()))
}
