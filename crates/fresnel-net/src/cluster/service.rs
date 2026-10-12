//! The path of one request through the layer.
//!
//! The steps, from the outside to the server: the time limit of the kind, the throttle of
//! the server, the token with the HTTP 401 rule, and the connection for the kind.

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::Full;
use tokio::time::{Instant, timeout_at};

use super::Inner;
use crate::body::{BodyLimits, ResponseBody};
use crate::channel::group::Group;
use crate::credential::layer;
use crate::error::NetError;
use crate::request::{Lane, Plan, RequestKind};

impl Inner {
    pub(super) async fn send(&self, mut request: Request<Bytes>) -> Result<Response<ResponseBody>, NetError> {
        let Some(kind) = request.extensions().get::<RequestKind>().copied() else {
            return Err(NetError::invalid("the request has no kind: set a `RequestKind` as its extension"));
        };
        let plan = kind.plan(&self.limits);
        self.address.apply(&mut request, &plan)?;
        let started = Instant::now();
        let deadline = started + plan.head;
        match timeout_at(deadline, self.answer(request, &plan, started, deadline)).await {
            Ok(answer) => answer,
            Err(_) => {
                let streams = self.group(plan.lane).map(stream_text).unwrap_or_default();
                Err(NetError::timeout(format!("{kind:?} request: no answer in {:?}{streams}", plan.head)))
            }
        }
    }

    /// Sends the request until an answer that is not a throttle, or until the repeats end.
    async fn answer(
        &self,
        request: Request<Bytes>,
        plan: &Plan,
        started: Instant,
        deadline: Instant,
    ) -> Result<Response<ResponseBody>, NetError> {
        let mut sends = 0;
        loop {
            self.throttle.gate(deadline).await?;
            let copy = layer::copy_of(&request);
            let response = match &self.credentials {
                Some(credentials) => {
                    layer::send(credentials, copy, |request| self.route(request, plan, started)).await?
                }
                None => self.route(copy, plan, started).await?,
            };
            // HTTP 429 is a definite refusal, so a second send is also safe for a write.
            let throttled = self.throttle.note(response.status(), response.headers());
            if !throttled || sends == self.throttle.repeats() {
                return Ok(response);
            }
            sends += 1;
        }
    }

    /// One send on the connection for the kind of the request.
    async fn route(
        &self,
        request: Request<Bytes>,
        plan: &Plan,
        started: Instant,
    ) -> Result<Response<ResponseBody>, NetError> {
        let request = request.map(Full::new);
        let Some(group) = self.group(plan.lane) else {
            return self.upgrade.send(request).await;
        };
        let (channel, stream) = group.stream().await;
        let (mut head, body) = channel.send(request).await?.into_parts();
        let limits = BodyLimits::new(plan.body_idle, plan.total, started);
        let body = ResponseBody::new(body).on_stream(stream).with_limits(limits).decoded(&mut head.headers);
        Ok(Response::from_parts(head, body))
    }

    fn group(&self, lane: Lane) -> Option<&Group> {
        match lane {
            Lane::Requests => Some(&self.requests),
            Lane::Watches => Some(&self.watches),
            Lane::Logs => Some(&self.logs),
            Lane::Upgrade => None,
        }
    }
}

/// The stream facts for the error of a request that waited for a free stream.
fn stream_text(group: &Group) -> String {
    let status = group.status();
    if status.waiting == 0 {
        return String::new();
    }
    let open: usize = status.channels.iter().map(|channel| channel.streams_open).sum();
    let budget: usize = status.channels.iter().map(|channel| channel.stream_budget).sum();
    let (connections, waiting) = (status.channels.len(), status.waiting);
    format!(
        " ({open} of {budget} streams are open on {connections} connections, {waiting} requests wait for a free stream)"
    )
}
