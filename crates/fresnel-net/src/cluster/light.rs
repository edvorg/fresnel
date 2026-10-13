//! The light requests of one cluster: its address and the token of its credential.

use std::sync::Arc;

use bytes::Bytes;
use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use http::{HeaderValue, Request};
use http_body_util::Full;

use super::address::Address;
use crate::connect::dial::Protocol;
use crate::credential::Credentials;
use crate::light::LightRequests;

pub(super) struct ClusterLight {
    pub(super) address: Arc<Address>,
    /// `None` for a cluster with no plugin. Its requests carry no token.
    pub(super) credentials: Option<Credentials>,
}

impl LightRequests for ClusterLight {
    fn build(&self, protocol: Protocol) -> Option<Request<Full<Bytes>>> {
        let authorization = match &self.credentials {
            Some(credentials) => Some(credentials.authorization_now()?),
            None => None,
        };
        Some(self.address.light_request(protocol, authorization))
    }

    fn token(&self) -> BoxFuture<'_, ()> {
        match &self.credentials {
            Some(credentials) => credentials.token_exists().boxed(),
            None => std::future::ready(()).boxed(),
        }
    }

    fn refused(&self, authorization: &HeaderValue) {
        if let Some(credentials) = &self.credentials {
            credentials.light_request_refused(authorization);
        }
    }
}
