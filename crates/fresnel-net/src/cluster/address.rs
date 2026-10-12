//! The URLs and the fixed headers of the requests of one cluster.

use bytes::Bytes;
use http::header::{ACCEPT_ENCODING, AUTHORIZATION, HOST, USER_AGENT};
use http::{HeaderValue, Request, Uri};
use http_body_util::Full;

use crate::connect::Endpoint;
use crate::connect::dial::Protocol;
use crate::error::NetError;
use crate::request::{Lane, Plan};

pub(super) struct Address {
    /// The host and the port, as they stand in a URL.
    authority: HeaderValue,
    /// The path before each API path. Empty for most clusters.
    base_path: String,
    user_agent: HeaderValue,
    light_path: String,
}

impl Address {
    pub(super) fn new(endpoint: &Endpoint, user_agent: &str, light_path: &str) -> Result<Self, NetError> {
        let host = if endpoint.host.contains(':') { format!("[{}]", endpoint.host) } else { endpoint.host.clone() };
        let authority = format!("{host}:{}", endpoint.port);
        let invalid = |what: &str| NetError::invalid(format!("the {what} of the cluster cannot stand in a request"));
        let address = Self {
            authority: HeaderValue::try_from(authority).map_err(|_| invalid("host"))?,
            base_path: endpoint.base_path.trim_end_matches('/').to_owned(),
            user_agent: HeaderValue::try_from(user_agent).map_err(|_| invalid("user agent"))?,
            light_path: light_path.to_owned(),
        };
        // The check of the fixed parts comes here, so that a light request cannot fail later.
        address.uri(Protocol::Http2, light_path)?;
        Ok(address)
    }

    /// The URL for a connection of one protocol. HTTP/2 needs the scheme and the host in the
    /// URL. HTTP/1.1 takes the path, and the host goes into a header.
    fn uri(&self, protocol: Protocol, path_and_query: &str) -> Result<Uri, NetError> {
        let (authority, base) = (self.authority.to_str().unwrap_or_default(), &self.base_path);
        let text = match protocol {
            Protocol::Http2 => format!("https://{authority}{base}{path_and_query}"),
            Protocol::Http1 => format!("{base}{path_and_query}"),
        };
        text.parse().map_err(|_| NetError::invalid(format!("`{path_and_query}` is not a path of a request")))
    }

    /// Gives the request of a caller the URL and the headers that its connection needs.
    pub(super) fn apply(&self, request: &mut Request<Bytes>, plan: &Plan) -> Result<(), NetError> {
        let protocol = if plan.lane == Lane::Upgrade { Protocol::Http1 } else { Protocol::Http2 };
        let path_and_query = request.uri().path_and_query().map_or("/", |path| path.as_str());
        *request.uri_mut() = self.uri(protocol, path_and_query)?;
        let headers = request.headers_mut();
        if protocol == Protocol::Http1 {
            headers.insert(HOST, self.authority.clone());
        }
        if !headers.contains_key(USER_AGENT) {
            headers.insert(USER_AGENT, self.user_agent.clone());
        }
        if plan.gzip && !headers.contains_key(ACCEPT_ENCODING) {
            headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip"));
        }
        Ok(())
    }

    /// The light request: a small read that each user can make.
    pub(super) fn light_request(&self, protocol: Protocol, authorization: Option<HeaderValue>) -> Request<Full<Bytes>> {
        let mut request = Request::new(Full::default());
        // `new` checked this URL. The start value of a request is the path `/`.
        if let Ok(uri) = self.uri(protocol, &self.light_path) {
            *request.uri_mut() = uri;
        }
        let headers = request.headers_mut();
        headers.insert(USER_AGENT, self.user_agent.clone());
        if protocol == Protocol::Http1 {
            headers.insert(HOST, self.authority.clone());
        }
        if let Some(authorization) = authorization {
            headers.insert(AUTHORIZATION, authorization);
        }
        request
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rustls::ClientConfig;

    use super::*;
    use crate::request::{Limits, RequestKind};

    fn address(host: &str, base_path: &str) -> Address {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        let endpoint = Endpoint {
            host: host.to_owned(),
            port: 6443,
            base_path: base_path.to_owned(),
            tls_server_name: None,
            tls: Arc::new(tls),
        };
        Address::new(&endpoint, "fresnel/test", "/version").expect("a good address")
    }

    fn applied(address: &Address, uri: &str, kind: RequestKind) -> Request<Bytes> {
        let mut request = Request::get(uri).body(Bytes::new()).expect("a request");
        address.apply(&mut request, &kind.plan(&Limits::default())).expect("a good path");
        request
    }

    #[test]
    fn a_channel_request_gets_the_full_url_and_gzip_only_for_a_read() {
        let address = address("api.example.test", "");
        let list = applied(&address, "/api/v1/pods?limit=500", RequestKind::ListPage);
        assert_eq!(list.uri(), "https://api.example.test:6443/api/v1/pods?limit=500");
        assert_eq!(list.headers()[ACCEPT_ENCODING], "gzip");
        assert_eq!(list.headers()[USER_AGENT], "fresnel/test");
        assert!(!list.headers().contains_key(HOST), "HTTP/2 takes the host from the URL");

        let from_kube = applied(&address, "https://other.test/api/v1/pods?watch=true", RequestKind::Watch);
        assert_eq!(
            from_kube.uri(),
            "https://api.example.test:6443/api/v1/pods?watch=true",
            "the cluster sets the host"
        );
        assert!(!from_kube.headers().contains_key(ACCEPT_ENCODING), "the events of a watch must arrive one by one");
        assert!(!applied(&address, "/api/v1/pods", RequestKind::Write).headers().contains_key(ACCEPT_ENCODING));
    }

    #[test]
    fn an_upgrade_request_gets_the_path_and_the_host_header() {
        let address = address("::1", "/k8s/clusters/c1/");
        let exec = applied(&address, "/api/v1/namespaces/a/pods/b/exec?command=sh", RequestKind::Upgrade);
        assert_eq!(exec.uri(), "/k8s/clusters/c1/api/v1/namespaces/a/pods/b/exec?command=sh");
        assert_eq!(exec.headers()[HOST], "[::1]:6443");

        let light = address.light_request(Protocol::Http2, Some(HeaderValue::from_static("Bearer t")));
        assert_eq!(light.uri(), "https://[::1]:6443/k8s/clusters/c1/version");
        assert_eq!(light.headers()[AUTHORIZATION], "Bearer t");
        assert_eq!(address.light_request(Protocol::Http1, None).uri(), "/k8s/clusters/c1/version");
    }
}
