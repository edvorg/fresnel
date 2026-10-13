//! The service between the kube client and the connection layer of one cluster.
//!
//! kube keeps its service in a `tower` buffer, and the buffer starts one task of its own. That
//! task hands each request to this service, and it is outside the start function of the app.
//! It cannot end before the last handle of the client drops: this service is always ready,
//! and its call only builds a future. The request itself runs in the task of its caller.

use std::error::Error;
use std::task::{Context, Poll};

use fresnel_net::ResponseBody;
use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use http::{Request, Response};
use kube::client::Body;
use tower_service::Service;

type BoxError = Box<dyn Error + Send + Sync>;

#[derive(Clone)]
pub(super) struct Bridge {
    cluster: fresnel_net::Cluster,
}

impl Bridge {
    pub(super) fn new(cluster: fresnel_net::Cluster) -> Self {
        Self { cluster }
    }
}

impl Service<Request<Body>> for Bridge {
    type Response = Response<ResponseBody>;
    type Error = BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, BoxError>>;

    /// An error here would stop the buffer of the kube client for each later request.
    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), BoxError>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let cluster = self.cluster.clone();
        async move {
            let (head, body) = request.into_parts();
            // The gateway gives each body as bytes in memory, so this read does not wait.
            let body = body.collect_bytes().await?;
            // The error keeps its type, so that the gateway reads its class again.
            Ok(cluster.send(Request::from_parts(head, body)).await?)
        }
        .boxed()
    }
}
