//! The task of the fake server and its connections: TLS, then HTTP/2 or HTTP/1.1.

use std::convert::Infallible;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use http::header::SEC_WEBSOCKET_KEY;
use http::{Request, StatusCode};
use hyper::body::Incoming;
use hyper::server::conn::{http1, http2};
use hyper::service::service_fn;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use socket2::{SockRef, TcpKeepalive};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio_rustls::server::TlsStream;

use super::answer::answer;
use super::pod::Peer;
use super::state::{Connection, Shared};
use crate::background::connection_end;

type Io = TokioIo<TlsStream<TcpStream>>;

/// The idle time before a keepalive probe of the system, and the time between two probes.
const PROBE: Duration = Duration::from_secs(1);

/// Accepts connections until an error. One task owns all connection tasks, so the end of
/// the server ends each connection.
pub(super) async fn run(shared: Arc<Shared>) -> io::Result<()> {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = shared.listener.accept() => {
                let (tcp, _) = accepted?;
                connections.spawn(serve(tcp, shared.admit()));
            }
            Some(ended) = connections.join_next() => connection_end(ended)?,
        }
    }
}

/// Serves one connection. An error here is most often the client: a reset or a bad
/// handshake. Thus it is a count and not a fault of the kit.
async fn serve(tcp: TcpStream, connection: Connection) {
    let ended = serve_protocol(&connection.shared, tcp, connection.number).await;
    connection.shared.count(|counters| counters.broken += u64::from(ended.is_err()));
}

/// The client selects the protocol in the TLS handshake. A client with no selection gets
/// HTTP/1.1.
///
/// A client that closes while data is on its way sends a reset, and the loopback of macOS
/// can lose that reset (see `proxy/drain.rs`). The server would then hold the connection
/// with no end, and a test that waits for its close would fail. So the system sends a
/// probe on each idle connection: the answer to a probe for a closed socket is a reset.
async fn serve_protocol(shared: &Arc<Shared>, tcp: TcpStream, number: u64) -> io::Result<()> {
    tcp.set_nodelay(true)?;
    SockRef::from(&tcp).set_tcp_keepalive(&TcpKeepalive::new().with_time(PROBE).with_interval(PROBE))?;
    let tls = shared.tls.accept(tcp).await?;
    let http2 = tls.get_ref().1.alpn_protocol() == Some(b"h2");
    let io = TokioIo::new(tls);
    if http2 { serve_http2(shared, io, number).await } else { serve_http1(shared, io, number).await }
}

/// The futures that hyper starts for the streams of one HTTP/2 connection.
///
/// hyper gives them to an executor. This one hands them to the task of the connection, so
/// that the kit starts no task outside its start function, and a stream ends with its connection.
#[derive(Clone, Default)]
struct StreamFutures {
    started: Arc<Mutex<Vec<BoxFuture<'static, ()>>>>,
    added: Arc<Notify>,
}

impl<F> hyper::rt::Executor<F> for StreamFutures
where
    F: Future<Output = ()> + Send + 'static,
{
    fn execute(&self, future: F) {
        self.started.lock().unwrap_or_else(PoisonError::into_inner).push(Box::pin(future));
        self.added.notify_one();
    }
}

impl StreamFutures {
    fn take(&self) -> Vec<BoxFuture<'static, ()>> {
        std::mem::take(&mut *self.started.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

async fn serve_http2(shared: &Arc<Shared>, io: Io, number: u64) -> io::Result<()> {
    let streams = StreamFutures::default();
    let last_request = Arc::new(Notify::new());
    let service = service_fn(|request: Request<Incoming>| {
        let (shared, last_request) = (Arc::clone(shared), Arc::clone(&last_request));
        async move {
            let (response, ends) = answer(&shared, number, request).await;
            if ends {
                last_request.notify_one();
            }
            Ok::<_, Infallible>(response)
        }
    });
    let mut builder = http2::Builder::new(streams.clone());
    builder.max_concurrent_streams(shared.stream_limit);
    let connection = builder.serve_connection(io, service);
    tokio::pin!(connection);
    let mut go_away = shared.go_away.subscribe();
    let mut running = FuturesUnordered::new();
    loop {
        tokio::select! {
            ended = &mut connection => return ended.map_err(io::Error::other),
            () = streams.added.notified() => running.extend(streams.take()),
            Some(()) = running.next() => {}
            // The connection takes no new stream and ends after the last open one.
            Ok(()) = go_away.changed() => connection.as_mut().graceful_shutdown(),
            () = last_request.notified() => connection.as_mut().graceful_shutdown(),
        }
    }
}

async fn serve_http1(shared: &Arc<Shared>, io: Io, number: u64) -> io::Result<()> {
    let upgrade = Mutex::new(None);
    let service = service_fn(|mut request: Request<Incoming>| {
        let on_upgrade = hyper::upgrade::on(&mut request);
        let websocket = request.headers().contains_key(SEC_WEBSOCKET_KEY);
        let peer = websocket.then(|| Peer::select(shared, request.uri(), request.headers()));
        let (shared, upgrade) = (Arc::clone(shared), &upgrade);
        async move {
            let (response, _ends) = answer(&shared, number, request).await;
            if response.status() == StatusCode::SWITCHING_PROTOCOLS {
                *upgrade.lock().unwrap_or_else(PoisonError::into_inner) = Some((on_upgrade, peer));
            }
            Ok::<_, Infallible>(response)
        }
    });
    http1::Builder::new().serve_connection(io, service).with_upgrades().await.map_err(io::Error::other)?;
    let upgraded = upgrade.lock().unwrap_or_else(PoisonError::into_inner).take();
    let Some((on_upgrade, peer)) = upgraded else { return Ok(()) };
    let upgraded = on_upgrade.await.map_err(io::Error::other)?;
    shared.count(|counters| counters.upgrades += 1);
    match peer {
        None => echo(upgraded).await,
        Some(Some(peer)) => peer.run(Arc::clone(shared), upgraded).await.map_err(io::Error::other),
        // The pod speaks none of the subprotocols, and the client refuses the answer.
        Some(None) => Ok(()),
    }
}

/// After an upgrade with no WebSocket, the connection is a raw stream. The server sends each
/// byte back, so that a test can see that the stream works in the two directions.
async fn echo(upgraded: Upgraded) -> io::Result<()> {
    let (mut read, mut write) = tokio::io::split(TokioIo::new(upgraded));
    tokio::io::copy(&mut read, &mut write).await?;
    write.shutdown().await
}
