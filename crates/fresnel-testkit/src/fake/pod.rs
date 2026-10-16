//! The WebSocket peer of the fake server: a pod with a shell and with ports, behind the
//! channel protocols of the API server (exec, attach and port forward).
//!
//! - Exec and attach: each byte of the input comes back on the standard output. A line
//!   `exit N` ends the process with the status of the exit code N; the end of the input of
//!   v5 ends it with 0. A size of the terminal comes back on channel 2, as `size {"Width":…}`.
//! - Port forward: each port of the request gets its two channels, with the port at their
//!   start, and each byte of its data comes back. The port 1 is closed: its error channel
//!   tells it, as a node does.
//!
//! The end of a process or of a port closes the TLS connection with no close message, as
//! the API server does. While the test holds the streams, the peer reads nothing: a pod
//! that reads no input, whose buffers are full, so that no ping is answered either.

use std::sync::Arc;

use bytes::{BufMut as _, Bytes, BytesMut};
use futures_util::{SinkExt as _, StreamExt as _};
use http::header::SEC_WEBSOCKET_PROTOCOL;
use http::{HeaderMap, Uri};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use tokio::io::AsyncWriteExt as _;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use super::state::Shared;

/// The subprotocols of the fake pod. A port forward has only v4, as a node.
pub(super) const V5: &str = "v5.channel.k8s.io";
pub(super) const V4: &str = "v4.channel.k8s.io";
/// The port that refuses each connection.
const CLOSED_PORT: u16 = 1;

type Socket = WebSocketStream<TokioIo<Upgraded>>;

/// What the peer of one upgraded connection does.
#[derive(Clone, Debug)]
pub(super) struct Peer {
    uri: Uri,
    protocol: &'static str,
}

impl Peer {
    /// The peer for a WebSocket request: the first subprotocol of the request that the pod
    /// speaks for its path. `None` when it speaks none of them.
    pub(super) fn select(shared: &Shared, uri: &Uri, headers: &HeaderMap) -> Option<Self> {
        let offered = headers.get(SEC_WEBSOCKET_PROTOCOL)?.to_str().ok()?;
        let speaks = |protocol: &str| match is_forward(uri) {
            true => protocol == V4,
            false => shared.exec_protocols().contains(&protocol),
        };
        let protocol = offered.split(',').map(str::trim).find(|protocol| speaks(protocol))?;
        let protocol = if protocol == V5 { V5 } else { V4 };
        Some(Self { uri: uri.clone(), protocol })
    }

    pub(super) fn protocol(&self) -> &'static str {
        self.protocol
    }

    /// Runs the peer to its end. An error of the socket is most often the client.
    pub(super) async fn run(self, shared: Arc<Shared>, upgraded: Upgraded) -> Result<(), WsError> {
        let mut socket = WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
        match is_forward(&self.uri) {
            true => forward(&shared, &mut socket, &ports(&self.uri)).await?,
            false => shell(&shared, &mut socket, self.protocol == V5).await?,
        }
        // The answer to a close of the client goes out; after it, the protocol has ended.
        match socket.flush().await {
            Ok(()) | Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => {}
            Err(error) => return Err(error),
        }
        // The end of the process or of the port: the server closes the TLS connection.
        socket.get_mut().shutdown().await?;
        Ok(())
    }
}

fn is_forward(uri: &Uri) -> bool {
    uri.path().ends_with("/portforward")
}

fn ports(uri: &Uri) -> Vec<u16> {
    let query = uri.query().unwrap_or_default();
    let ports = query.split('&').find_map(|pair| pair.strip_prefix("ports=")).unwrap_or_default();
    ports.split("%2C").flat_map(|part| part.split(',')).filter_map(|port| port.parse().ok()).collect()
}

fn message(channel: u8, data: &[u8]) -> Message {
    let mut message = BytesMut::with_capacity(1 + data.len());
    message.put_u8(channel);
    message.extend_from_slice(data);
    Message::Binary(message.freeze())
}

/// The next message of the client. While the test holds the streams, the peer reads
/// nothing. `None` at the end or the close of the client.
async fn next(shared: &Shared, socket: &mut Socket) -> Option<Result<Bytes, WsError>> {
    loop {
        shared.until_streams_flow().await;
        let read = tokio::select! {
            read = socket.next() => read?,
            () = shared.until_streams_held() => continue,
        };
        match read {
            Ok(Message::Binary(data)) => return Some(Ok(data)),
            Ok(Message::Close(_)) => return None,
            Ok(_) => {}
            Err(error) => return Some(Err(error)),
        }
    }
}

async fn shell(shared: &Shared, socket: &mut Socket, v5: bool) -> Result<(), WsError> {
    // The server opens the standard output with an empty message.
    socket.send(message(1, &[])).await?;
    let mut line = Vec::new();
    while let Some(data) = next(shared, socket).await {
        let data = data?;
        match (data.first(), data.get(1..).unwrap_or_default()) {
            (Some(0), input) => {
                socket.send(message(1, input)).await?;
                for byte in input {
                    line.push(*byte);
                    if *byte == b'\n' {
                        if let Some(code) = exit_code(&line) {
                            return socket.send(message(3, status(code).as_bytes())).await;
                        }
                        line.clear();
                    }
                }
            }
            (Some(4), size) => socket.send(message(2, &[b"size ", size, b"\n"].concat())).await?,
            (Some(255), [0]) if v5 => return socket.send(message(3, status(0).as_bytes())).await,
            _ => {}
        }
    }
    Ok(())
}

/// The exit code of a line `exit N`.
fn exit_code(line: &[u8]) -> Option<i32> {
    std::str::from_utf8(line).ok()?.trim().strip_prefix("exit ")?.parse().ok()
}

/// The status of a process, as the API server writes it.
fn status(code: i32) -> String {
    if code == 0 {
        return r#"{"metadata":{},"status":"Success"}"#.to_owned();
    }
    format!(
        r#"{{"metadata":{{}},"status":"Failure","message":"command terminated with non-zero exit code: exit code {code}","reason":"NonZeroExitCode","details":{{"causes":[{{"reason":"ExitCode","message":"{code}"}}]}}}}"#
    )
}

async fn forward(shared: &Shared, socket: &mut Socket, ports: &[u16]) -> Result<(), WsError> {
    for (data, port) in (0_u8..).step_by(2).zip(ports) {
        socket.send(message(data, &port.to_le_bytes())).await?;
        socket.send(message(data + 1, &port.to_le_bytes())).await?;
        if *port == CLOSED_PORT {
            let text =
                format!("error forwarding port {port} to pod web: dial tcp4 127.0.0.1:{port}: connection refused");
            return socket.send(message(data + 1, text.as_bytes())).await;
        }
    }
    while let Some(data) = next(shared, socket).await {
        let data = data?;
        if let (Some(&channel), Some(bytes)) = (data.first(), data.get(1..))
            && channel % 2 == 0
        {
            socket.send(message(channel, bytes)).await?;
        }
    }
    Ok(())
}
