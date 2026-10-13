//! The task of the fake server and its connections: TLS, then HTTP/2 or HTTP/1.1.

use std::convert::Infallible;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use http::{Request, StatusCode};
use hyper::body::Incoming;
use hyper::server::conn::{http1, http2};
use hyper::service::service_fn;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio_rustls::server::TlsStream;

use super::answer::answer;
use super::state::{Connection, Shared};
use crate::background::connection_end;

type Io = TokioIo<TlsStream<TcpStream>>;

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
async fn serve_protocol(shared: &Arc<Shared>, tcp: TcpStream, number: u64) -> io::Result<()> {
    tcp.set_nodelay(true)?;
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
        let (shared, upgrade) = (Arc::clone(shared), &upgrade);
        async move {
            let (response, _ends) = answer(&shared, number, request).await;
            if response.status() == StatusCode::SWITCHING_PROTOCOLS {
                *upgrade.lock().unwrap_or_else(PoisonError::into_inner) = Some(on_upgrade);
            }
            Ok::<_, Infallible>(response)
        }
    });
    http1::Builder::new().serve_connection(io, service).with_upgrades().await.map_err(io::Error::other)?;
    let on_upgrade = upgrade.lock().unwrap_or_else(PoisonError::into_inner).take();
    match on_upgrade {
        Some(on_upgrade) => echo(shared, on_upgrade.await.map_err(io::Error::other)?).await,
        None => Ok(()),
    }
}

/// After the upgrade, the connection is a raw stream. The server sends each byte back, so
/// that a test can see that the stream works in the two directions.
async fn echo(shared: &Shared, upgraded: Upgraded) -> io::Result<()> {
    shared.count(|counters| counters.upgrades += 1);
    let (mut read, mut write) = tokio::io::split(TokioIo::new(upgraded));
    tokio::io::copy(&mut read, &mut write).await?;
    write.shutdown().await
}
