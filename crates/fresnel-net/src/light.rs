//! The light request of a cluster: a small read that each user can make (plan section 8.5).
//!
//! A channel sends it to prove a new connection, to keep an idle connection open and to
//! test a connection after a wake. A warm upgrade connection gets the same test.
//!
//! On a cluster with a credential plugin, the light request carries the token, and none
//! goes out while the cluster has no token. A kube-apiserver ends an HTTP/2 connection
//! after its answer to a request with no credential. In milestone M0, a plugin that hung
//! for 20 s cost 43 connections for that reason, and the minimum is 3.
//!
//! The server ends the connection in the same way after HTTP 401 to a token that it
//! refuses. Thus the HTTP 401 of a light request goes to the credential of the cluster, as
//! the HTTP 401 of a request of a caller does. On k3s, a refused token with no such report
//! cost 51 connections in 30 s, and the credential showed as valid.

use bytes::Bytes;
use futures_util::future::BoxFuture;
use http::{HeaderValue, Request};
use http_body_util::Full;

use crate::connect::dial::Protocol;

/// Builds the light requests of one cluster.
pub(crate) trait LightRequests: Send + Sync + 'static {
    /// A light request for a connection of one protocol, with the token of this moment.
    /// `None` while the cluster has a credential plugin and no token: the caller sends
    /// nothing then.
    fn build(&self, protocol: Protocol) -> Option<Request<Full<Bytes>>>;

    /// Waits until the cluster has a token. For a cluster with no plugin, the wait ends at once.
    fn token(&self) -> BoxFuture<'_, ()>;

    /// The server answered HTTP 401 to a light request with this `Authorization` value. The
    /// credential then decides as after a request of a caller: a plugin run, or a proof.
    fn refused(&self, authorization: &HeaderValue);
}
