//! One connect attempt: TCP, then TLS with exactly one protocol.
//!
//! The API server offers HTTP/2 and HTTP/1.1. Each connection asks for exactly one of them,
//! so that the server cannot select HTTP/2 for a connection that must carry an upgrade.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use socket2::{SockRef, TcpKeepalive};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

/// The protocol of one connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Protocol {
    /// For a channel.
    Http2,
    /// For an upgrade connection.
    Http1,
}

impl Protocol {
    fn alpn(self) -> &'static [u8] {
        match self {
            Self::Http2 => b"h2",
            Self::Http1 => b"http/1.1",
        }
    }
}

/// A byte stream to the API server after the TLS handshake.
pub(crate) trait Stream: AsyncRead + AsyncWrite + Send + Unpin + 'static {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin + 'static> Stream for T {}

/// One connection. The type is a box, so that a test can give a stream in memory.
pub(crate) type Io = Box<dyn Stream>;

/// The kernel reports a dead idle connection after 60 s. This is the last detector for an
/// upgraded connection, which has no HTTP/2 ping.
const KEEPALIVE_IDLE: Duration = Duration::from_secs(30);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
const KEEPALIVE_PROBES: u32 = 3;

/// The TLS side of the connects of one cluster.
pub(crate) struct Tls {
    name: ServerName<'static>,
    http2: TlsConnector,
    http1: TlsConnector,
}

impl Tls {
    pub(crate) fn new(config: &ClientConfig, name: ServerName<'static>) -> Self {
        let with = |protocol: Protocol| {
            let mut config = config.clone();
            config.alpn_protocols = vec![protocol.alpn().to_vec()];
            TlsConnector::from(Arc::new(config))
        };
        Self { name, http2: with(Protocol::Http2), http1: with(Protocol::Http1) }
    }

    /// Connects to one address. A connection counts only after the TLS handshake, because a
    /// held connection can stop before the first byte of the server.
    pub(crate) async fn attempt(&self, address: SocketAddr, protocol: Protocol) -> io::Result<Io> {
        let tcp = TcpStream::connect(address).await?;
        // A ping and a key of a terminal must not wait for more data.
        tcp.set_nodelay(true)?;
        let keepalive = TcpKeepalive::new()
            .with_time(KEEPALIVE_IDLE)
            .with_interval(KEEPALIVE_INTERVAL)
            .with_retries(KEEPALIVE_PROBES);
        SockRef::from(&tcp).set_tcp_keepalive(&keepalive)?;
        let connector = match protocol {
            Protocol::Http2 => &self.http2,
            Protocol::Http1 => &self.http1,
        };
        let tls = connector.connect(self.name.clone(), tcp).await?;
        if tls.get_ref().1.alpn_protocol() != Some(protocol.alpn()) {
            return Err(io::Error::other(format!("the server did not select {protocol:?}")));
        }
        Ok(Box::new(tls))
    }
}
