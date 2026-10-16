//! The tunnel of one local connection of a port forward: one socket to one port of a pod
//! (plan section 12.3). The forward of M5 opens one tunnel for each local connection.
//!
//! The WebSocket form of the port forward (`portforward.k8s.io` in its SPDY form) has the
//! subprotocol `v4.channel.k8s.io` and two channels for each port of the request: its data
//! (0) and its error (1). The server starts each channel with the port, two bytes in little
//! endian. The node dials the port in the pod at once, and writes a failure on the error
//! channel, for example "connection refused", before the end of the socket.

use std::sync::Arc;

use bytes::Bytes;
use fresnel_net::{Socket, SocketEnd, SocketSpec, SocketState, Writer};

use super::channel::{self, V4};
use super::journaled;
use crate::cluster::{ClusterId, Gateway};
use crate::error::CoreError;
use crate::journal::Journal;
use crate::kinds::Kind;
use crate::store::ObjectKey;

/// A tunnel has one port: one TCP connection in the pod.
const PROTOCOLS: &[&str] = &[V4];
const DATA: u8 = 0;
const ERROR: u8 = 1;
/// The most bytes of the error text that a tunnel keeps: the node writes one line.
const MAX_ERROR: usize = 4096;

/// How a tunnel ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TunnelEnd {
    /// The socket ended with no error of the node, for example after the pod closed the connection.
    Socket(SocketEnd),
    /// The node could not reach the port or lost it: its text.
    Refused(String),
    /// The server broke the protocol, and the app closed the socket.
    Invalid(String),
}

/// One tunnel to one port of a pod. Its socket closes when this value drops.
pub struct Tunnel {
    socket: Socket,
    port: u16,
    data_port: Prefix,
    error_port: Prefix,
    error: Vec<u8>,
    invalid: Option<String>,
    what: Arc<str>,
    cluster: ClusterId,
    journal: Journal,
}

impl Tunnel {
    /// Opens a tunnel to `port` of the pod on a warm or a new upgrade connection. The journal
    /// gets each abnormal end of its socket; a forward with many tunnels tells the rest.
    pub async fn open(gateway: &Gateway, pod: &ObjectKey, port: u16, journal: Journal) -> Result<Self, CoreError> {
        if pod.namespace.is_empty() {
            return Err(CoreError::invalid("a pod is in a namespace, and the port forward has none"));
        }
        let what = format!("a port forward to pod {}/{} port {port}", pod.namespace, pod.name);
        let cluster = gateway.cluster().clone();
        let request =
            kube::core::Request::new(Kind::Pod.info().path_in(&pod.namespace)).portforward(&pod.name, &[port]);
        let request = request.map_err(|error| CoreError::invalid(format!("the request of {what}: {error}")))?;
        let spec = SocketSpec {
            role: "port forward socket",
            protocols: PROTOCOLS,
            on_end: Some(journaled(&journal, &cluster, &what)),
        };
        let socket = gateway.upgrade(request, spec).await?;
        Ok(Self {
            socket,
            port,
            data_port: Prefix::default(),
            error_port: Prefix::default(),
            error: Vec::new(),
            invalid: None,
            what: what.into(),
            cluster,
            journal,
        })
    }

    /// The handle that sends data to the port.
    pub fn writer(&self) -> TunnelWriter {
        TunnelWriter { writer: self.socket.writer() }
    }

    /// The next data of the port. `None` after the end, which [`Tunnel::end`] then tells.
    pub async fn next(&mut self) -> Option<Bytes> {
        loop {
            let message = self.socket.recv().await?;
            let Some((number, data)) = channel::split(&message) else { continue };
            let prefix = match number {
                DATA => &mut self.data_port,
                ERROR => &mut self.error_port,
                other => {
                    self.refuse(format!("the server sent data on channel {other}, and a tunnel has two"));
                    continue;
                }
            };
            let data = match prefix.strip(data, self.port) {
                Ok(data) => data,
                Err(text) => {
                    self.refuse(text);
                    continue;
                }
            };
            match number {
                _ if data.is_empty() || self.invalid.is_some() => {}
                DATA => return Some(data),
                _ => {
                    let room = MAX_ERROR.saturating_sub(self.error.len());
                    self.error.extend(data.iter().take(room));
                }
            }
        }
    }

    /// The end, once the socket ended.
    pub fn end(&self) -> Option<TunnelEnd> {
        if let Some(text) = &self.invalid {
            return Some(TunnelEnd::Invalid(text.clone()));
        }
        let SocketState::Ended(end) = self.socket.state() else { return None };
        if self.error.is_empty() {
            return Some(TunnelEnd::Socket(end));
        }
        Some(TunnelEnd::Refused(String::from_utf8_lossy(&self.error).into_owned()))
    }

    /// The state of the socket.
    pub fn state(&self) -> SocketState {
        self.socket.state()
    }

    /// Closes the socket after the data that waits. The protocol has no half close: the pod
    /// gets the end of its connection only with the end of the socket.
    pub fn close(&self) {
        self.socket.close();
    }

    fn refuse(&mut self, text: String) {
        if self.invalid.is_none() {
            self.journal.warning(self.cluster.as_str(), format!("{}: {text}", self.what));
            self.invalid = Some(text);
            self.socket.close();
        }
    }
}

impl std::fmt::Debug for Tunnel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tunnel").field("what", &self.what).field("socket", &self.socket).finish_non_exhaustive()
    }
}

/// Sends the data of a local connection to the port. A clone sends to the same tunnel.
#[derive(Clone, Debug)]
pub struct TunnelWriter {
    writer: Writer,
}

impl TunnelWriter {
    /// Sends bytes to the port. It waits while the queue of the socket is full: a pod that
    /// reads slowly slows the local client, and that wait is no break.
    pub async fn send(&self, data: &[u8]) -> Result<(), CoreError> {
        channel::send(&self.writer, DATA, data).await
    }
}

/// The two bytes of the port at the start of the data of one channel.
#[derive(Debug, Default)]
struct Prefix(Vec<u8>);

impl Prefix {
    /// Takes the bytes of the port from the start of `data` and gives the rest. The port
    /// must be the one of the request.
    fn strip(&mut self, data: Bytes, port: u16) -> Result<Bytes, String> {
        let take = 2_usize.saturating_sub(self.0.len()).min(data.len());
        self.0.extend(data.iter().take(take));
        if take > 0
            && let [low, high] = self.0[..]
            && u16::from_le_bytes([low, high]) != port
        {
            let sent = u16::from_le_bytes([low, high]);
            return Err(format!("the server started a channel with the port {sent}, not {port}"));
        }
        Ok(data.slice(take..))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_port_at_the_start_of_a_channel_goes_and_the_data_after_it_stays() {
        let mut prefix = Prefix::default();
        assert_eq!(prefix.strip(Bytes::from_static(&[0x90, 0x1f]), 8080), Ok(Bytes::new()), "the first message");
        assert_eq!(prefix.strip(Bytes::from_static(b"HTTP/1.0"), 8080), Ok(Bytes::from_static(b"HTTP/1.0")));

        let mut split = Prefix::default();
        assert_eq!(split.strip(Bytes::from_static(&[0x90]), 8080), Ok(Bytes::new()), "a port in two messages");
        assert_eq!(split.strip(Bytes::from_static(&[0x1f, b'a']), 8080), Ok(Bytes::from_static(b"a")));

        let mut other = Prefix::default();
        let text = "the server started a channel with the port 80, not 8080";
        assert_eq!(other.strip(Bytes::from_static(&[80, 0, b'a']), 8080), Err(text.to_owned()));
    }
}
