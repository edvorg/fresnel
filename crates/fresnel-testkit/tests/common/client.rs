//! A small HTTP client over TLS for the tests of the kit. Each test holds its connections,
//! so that it can count them and see the end of each one.

#![allow(dead_code, reason = "each test file uses a part of this module")]

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Empty};
use hyper::body::Incoming;
use hyper::client::conn::{http1, http2};
use hyper::upgrade::Upgraded;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

/// The longest wait for a signal that must arrive.
pub(crate) const LIMIT: Duration = Duration::from_secs(20);

pub(crate) async fn in_time<T>(signal: impl Future<Output = T>) -> T {
    timeout(LIMIT, signal).await.expect("the signal arrives in time")
}

/// The stream limit of a client before the settings of the server arrive. hyper starts with
/// 100, which is also the limit of a kube-apiserver. With an other number, a test can see
/// that a limit came from the server.
pub(crate) const STREAMS_BEFORE_SETTINGS: usize = 7;

type Io = TokioIo<TlsStream<TcpStream>>;

/// The TLS settings of a client: the certificates that it trusts and, if the server asks
/// for one, its own certificate.
pub(crate) struct Trust {
    pub(crate) roots: Vec<CertificateDer<'static>>,
    pub(crate) identity: Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>,
}

impl Trust {
    /// Connects with TCP and TLS and offers exactly one protocol, as the app does.
    pub(crate) async fn connect(&self, server: SocketAddr, protocol: &str) -> TlsStream<TcpStream> {
        let mut roots = RootCertStore::empty();
        for root in &self.roots {
            roots.add(root.clone()).expect("a certificate");
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_root_certificates(roots);
        let mut config = match &self.identity {
            Some((chain, key)) => builder.with_client_auth_cert(chain.clone(), key.clone_key()).expect("a client key"),
            None => builder.with_no_client_auth(),
        };
        config.alpn_protocols = vec![protocol.as_bytes().to_vec()];
        let tcp = in_time(TcpStream::connect(server)).await.expect("the TCP connect");
        let name = ServerName::from(server.ip());
        in_time(TlsConnector::from(Arc::new(config)).connect(name, tcp)).await.expect("the TLS handshake")
    }

    pub(crate) async fn http2(&self, server: SocketAddr) -> Http2 {
        let io = TokioIo::new(self.connect(server, "h2").await);
        let mut builder = http2::Builder::new(TokioExecutor::new());
        builder.initial_max_send_streams(STREAMS_BEFORE_SETTINGS);
        let (sender, connection) = in_time(builder.handshake(io)).await.expect("HTTP/2");
        Http2 { sender, connection: Box::pin(connection) }
    }

    pub(crate) async fn http1(&self, server: SocketAddr) -> Http1 {
        let io = TokioIo::new(self.connect(server, "http/1.1").await);
        let (sender, connection) = in_time(http1::handshake(io)).await.expect("HTTP/1.1");
        Http1 { sender, connection: Box::pin(connection.with_upgrades()) }
    }
}

pub(crate) fn get(path: &str) -> Request<Empty<Bytes>> {
    Request::get(path).header("host", "kit").body(Empty::new()).expect("a request")
}

/// One HTTP/2 connection. hyper needs a poll of `connection` to send a request.
pub(crate) struct Http2 {
    pub(crate) sender: http2::SendRequest<Empty<Bytes>>,
    pub(crate) connection: Pin<Box<http2::Connection<Io, Empty<Bytes>, TokioExecutor>>>,
}

impl Http2 {
    /// Sends the request and waits for the head of the response.
    pub(crate) async fn send(&mut self, request: Request<Empty<Bytes>>) -> hyper::Result<Response<Incoming>> {
        tokio::select! {
            response = self.sender.send_request(request) => response,
            ended = &mut self.connection => panic!("the connection ended before the response: {ended:?}"),
        }
    }

    pub(crate) async fn get(&mut self, path: &str) -> Response<Incoming> {
        in_time(self.send(get(path))).await.expect("a response")
    }

    /// Tries a request. `None` when the connection takes no request: after GOAWAY, hyper
    /// ends the connection future or refuses the request.
    pub(crate) async fn try_get(&mut self, path: &str) -> Option<Response<Incoming>> {
        tokio::select! {
            response = self.sender.send_request(get(path)) => response.ok(),
            _ended = &mut self.connection => None,
        }
    }

    /// The stream limit of the server. It is known after the first response.
    pub(crate) fn stream_limit(&self) -> usize {
        self.connection.current_max_send_streams()
    }
}

/// One HTTP/1.1 connection. hyper needs a poll of `connection` for each read and write.
pub(crate) struct Http1 {
    pub(crate) sender: http1::SendRequest<Empty<Bytes>>,
    pub(crate) connection: Pin<Box<dyn Future<Output = hyper::Result<()>> + Send>>,
}

impl Http1 {
    /// Sends the request and reads the full response.
    pub(crate) async fn fetch(&mut self, request: Request<Empty<Bytes>>) -> (http::response::Parts, Bytes) {
        let exchange = async {
            let (head, body) = self.sender.send_request(request).await.expect("a response").into_parts();
            (head, body.collect().await.expect("a body").to_bytes())
        };
        in_time(async {
            tokio::select! {
                response = exchange => response,
                ended = &mut self.connection => panic!("the connection ended before the response: {ended:?}"),
            }
        })
        .await
    }

    /// Sends a request that asks for an upgrade. Gives the raw stream that follows HTTP 101.
    /// The connection future of hyper ends when it hands the stream over.
    pub(crate) async fn upgrade(mut self, request: Request<Empty<Bytes>>) -> TokioIo<Upgraded> {
        let exchange = async { tokio::join!(self.sender.send_request(request), &mut self.connection) };
        let (response, ended) = in_time(exchange).await;
        ended.expect("the connection hands its stream over");
        let response = response.expect("a response");
        assert_eq!(response.status(), http::StatusCode::SWITCHING_PROTOCOLS);
        assert_eq!(response.headers().get(http::header::UPGRADE).expect("the protocol"), "websocket");
        TokioIo::new(in_time(hyper::upgrade::on(response)).await.expect("the raw stream"))
    }
}

/// Reads the next event line of a watch stream. `None` is the clean end of the stream.
/// HTTP/2 can end a stream with an empty data frame, which is not a line.
pub(crate) async fn next_line(events: &mut Incoming) -> Option<String> {
    loop {
        let frame = in_time(events.frame()).await?.expect("a frame and not an error");
        let data = frame.into_data().expect("data");
        if !data.is_empty() {
            return Some(String::from_utf8(data.to_vec()).expect("text"));
        }
    }
}

/// Reads the full body of a response.
pub(crate) async fn text(response: Response<Incoming>) -> String {
    let body = in_time(response.into_body().collect()).await.expect("a body").to_bytes();
    String::from_utf8(body.to_vec()).expect("text")
}
