//! Exec and attach: a process of a container on one socket (plan section 12.8).
//!
//! The terminal of M10 draws the output and gives the keys. This module carries the bytes:
//! the standard streams, the size of the terminal, the end of the input of v5 and the exit.

use std::sync::Arc;

use bytes::Bytes;
use fresnel_net::{Socket, SocketEnd, SocketSpec, SocketState, Writer};
use http::Request;
use kube::core::subresource::AttachParams;
use tokio::sync::watch;

use super::channel::{self, CLOSE, ERROR, RESIZE, STDERR, STDIN, STDOUT, V4, V5};
use super::journaled;
use super::status::{self, Exit};
use crate::cluster::{ClusterId, Gateway};
use crate::error::CoreError;
use crate::journal::Journal;
use crate::kinds::Kind;
use crate::store::ObjectKey;

/// The subprotocols of an exec, the preferred one first: v4 for a server before 1.30.
const PROTOCOLS: &[&str] = &[V5, V4];

/// A process to run in a container, or the main process of a container to attach to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exec {
    /// The namespace and the name of the pod.
    pub pod: ObjectKey,
    /// `None` for the default container of the pod.
    pub container: Option<String>,
    /// The program and its arguments. `None` attaches to the main process of the container.
    pub command: Option<Vec<String>>,
    /// A terminal: the output has one stream, and a resize reaches the process.
    pub tty: bool,
    /// The process reads the input of the app.
    pub stdin: bool,
}

impl Exec {
    /// The request of the exec or the attach.
    pub(super) fn request(&self) -> Result<Request<Vec<u8>>, CoreError> {
        if self.pod.namespace.is_empty() {
            return Err(CoreError::invalid("a pod is in a namespace, and the exec has none"));
        }
        // With a terminal, the server joins the error output to the standard output.
        let params = AttachParams {
            container: self.container.clone(),
            stdin: self.stdin,
            stdout: true,
            stderr: !self.tty,
            tty: self.tty,
            ..AttachParams::default()
        };
        let pods = kube::core::Request::new(Kind::Pod.info().path_in(&self.pod.namespace));
        let request = match &self.command {
            Some(command) => pods.exec(&self.pod.name, command.clone(), &params),
            None => pods.attach(&self.pod.name, &params),
        };
        request.map_err(|error| CoreError::invalid(format!("the request of {}: {error}", self.what())))
    }

    /// The words for the journal. The command stays out: a line of the user can hold a password.
    fn what(&self) -> String {
        let (verb, pod) = (if self.command.is_some() { "an exec in" } else { "an attach to" }, &self.pod);
        match &self.container {
            Some(container) => format!("{verb} pod {}/{} (container {container})", pod.namespace, pod.name),
            None => format!("{verb} pod {}/{}", pod.namespace, pod.name),
        }
    }
}

/// A piece of the output of the process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// The standard output, and with a terminal also the error output.
    Stdout(Bytes),
    /// The error output of a process with no terminal.
    Stderr(Bytes),
}

/// How a session ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecEnd {
    /// The process ended: its status. The status decides, also when the socket broke after it.
    Exited(Exit),
    /// The socket ended with no status, for example because it went silent.
    Socket(SocketEnd),
    /// The server sent a message that the protocol does not have, and the app closed the socket.
    Invalid(String),
}

/// One running exec or attach. Its socket closes when this value drops.
pub struct ExecSession {
    socket: Socket,
    /// The server speaks v5, which can close the input.
    closes: bool,
    exit: Option<Exit>,
    invalid: Option<String>,
    what: Arc<str>,
    cluster: ClusterId,
    journal: Journal,
}

impl ExecSession {
    /// Opens the session on a warm or a new upgrade connection of the gateway. The journal
    /// gets its start, its exit and each abnormal end of its socket.
    pub async fn open(gateway: &Gateway, exec: &Exec, journal: Journal) -> Result<Self, CoreError> {
        let (what, cluster) = (exec.what(), gateway.cluster().clone());
        let on_end = journaled(&journal, &cluster, &what);
        let spec = SocketSpec { role: "exec socket", protocols: PROTOCOLS, on_end: Some(on_end) };
        let opened = match exec.request() {
            Ok(request) => gateway.upgrade(request, spec).await,
            Err(error) => Err(error),
        };
        let socket =
            opened.inspect_err(|error| journal.warning(cluster.as_str(), format!("{what}: no start: {error}")))?;
        journal.info(cluster.as_str(), format!("{what}: open, {}", socket.protocol()));
        let closes = socket.protocol() == V5;
        Ok(Self { socket, closes, exit: None, invalid: None, what: what.into(), cluster, journal })
    }

    /// The handle for the input and the size of the terminal.
    pub fn input(&self) -> Input {
        Input { writer: self.socket.writer(), closes: self.closes }
    }

    /// The next piece of output. `None` after the end, which [`ExecSession::end`] then tells.
    pub async fn next(&mut self) -> Option<Output> {
        loop {
            let message = self.socket.recv().await?;
            let Some((number, data)) = channel::split(&message) else { continue };
            match read(number, data, self.closes) {
                Read::Output(output) if self.invalid.is_none() => return Some(output),
                Read::Status(data) if self.exit.is_none() => self.exited(status::exit(&data)),
                Read::Output(_) | Read::Nothing => {}
                // A second status, or data on a channel that the server does not write.
                Read::Status(_) => self.refuse(ERROR),
                Read::Foreign(number) => self.refuse(number),
            }
        }
    }

    /// The end, once the process ended or the socket did.
    pub fn end(&self) -> Option<ExecEnd> {
        if let Some(text) = &self.invalid {
            return Some(ExecEnd::Invalid(text.clone()));
        }
        if let Some(exit) = &self.exit {
            return Some(ExecEnd::Exited(exit.clone()));
        }
        match self.socket.state() {
            SocketState::Ended(end) => Some(ExecEnd::Socket(end)),
            SocketState::Open | SocketState::WriteWaits { .. } | SocketState::Closing => None,
        }
    }

    /// The state of the socket: open, a write that waits, the close, or its end.
    pub fn state(&self) -> SocketState {
        self.socket.state()
    }

    /// A receiver of each change of the state, for a view that waits for one.
    pub fn states(&self) -> watch::Receiver<SocketState> {
        self.socket.states()
    }

    /// Closes the socket. The process of a terminal goes on in the pod: the terminal sends
    /// Ctrl-D first (terminal prototype, X11).
    pub fn close(&self) {
        self.socket.close();
    }

    fn exited(&mut self, exit: Exit) {
        let source = self.cluster.as_str();
        match &exit {
            Exit::Code(_) => self.journal.info(source, format!("{}: {exit}", self.what)),
            Exit::Error { .. } => self.journal.warning(source, format!("{}: {exit}", self.what)),
        }
        self.exit = Some(exit);
    }

    /// A message that the server never sends: the session ends, and its output stops there.
    fn refuse(&mut self, number: u8) {
        if self.invalid.is_none() {
            let text = format!("the server sent data on channel {number}, which it does not write");
            self.journal.warning(self.cluster.as_str(), format!("{}: {text}", self.what));
            self.invalid = Some(text);
            self.socket.close();
        }
    }
}

impl std::fmt::Debug for ExecSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecSession").field("what", &self.what).field("socket", &self.socket).finish_non_exhaustive()
    }
}

/// What one message of the server is for a session.
#[derive(Debug, PartialEq, Eq)]
enum Read {
    Output(Output),
    /// The data of the error channel: the status of the process.
    Status(Bytes),
    Nothing,
    /// Data on a channel that the server does not write.
    Foreign(u8),
}

/// The meaning of the data of one channel. `v5` has the close of a channel.
fn read(number: u8, data: Bytes, v5: bool) -> Read {
    match number {
        // The server opens a channel with an empty message.
        _ if data.is_empty() => Read::Nothing,
        STDOUT => Read::Output(Output::Stdout(data)),
        STDERR => Read::Output(Output::Stderr(data)),
        ERROR => Read::Status(data),
        // v5: the server closed one of its channels. The end of the socket follows.
        CLOSE if v5 => Read::Nothing,
        other => Read::Foreign(other),
    }
}

/// Sends the input and the size of the terminal of one session. A clone sends to the same one.
#[derive(Clone, Debug)]
pub struct Input {
    writer: Writer,
    closes: bool,
}

impl Input {
    /// Sends bytes to the standard input. It waits while the queue of the socket is full,
    /// for example for a process that reads nothing; that wait is no break.
    pub async fn send(&self, data: &[u8]) -> Result<(), CoreError> {
        channel::send(&self.writer, STDIN, data).await
    }

    /// The new size of the terminal, in columns and rows.
    pub async fn resize(&self, columns: u16, rows: u16) -> Result<(), CoreError> {
        let size = format!(r#"{{"Width":{columns},"Height":{rows}}}"#);
        Ok(self.writer.send(channel::message(RESIZE, size.as_bytes())).await?)
    }

    /// Ends the standard input, as the end of a pipe does. Only v5 can: v4 refuses it. A
    /// shell with a terminal does not end with it (terminal prototype, X11).
    pub async fn close_stdin(&self) -> Result<(), CoreError> {
        if !self.closes {
            return Err(CoreError::invalid("the server has the subprotocol v4, which cannot end the input"));
        }
        Ok(self.writer.send(Bytes::from_static(&[CLOSE, STDIN])).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_channel_of_the_server_has_its_meaning() {
        let data = || Bytes::from_static(b"ls\n");
        assert_eq!(read(STDOUT, data(), true), Read::Output(Output::Stdout(data())));
        assert_eq!(read(STDERR, data(), false), Read::Output(Output::Stderr(data())));
        assert_eq!(read(ERROR, data(), true), Read::Status(data()));
        assert_eq!(read(STDIN, Bytes::new(), true), Read::Nothing, "the empty message that opens a channel");
        assert_eq!((read(CLOSE, data(), true), read(CLOSE, data(), false)), (Read::Nothing, Read::Foreign(CLOSE)));
        assert_eq!(
            (read(STDIN, data(), true), read(RESIZE, data(), true)),
            (Read::Foreign(STDIN), Read::Foreign(RESIZE))
        );
    }

    #[test]
    fn the_request_has_the_streams_of_a_terminal_and_the_journal_no_command() {
        let exec = Exec {
            pod: ObjectKey::new("shop", "web-1"),
            container: Some("app".to_owned()),
            command: Some(vec!["mysql".to_owned(), "-psecret".to_owned()]),
            tty: true,
            stdin: true,
        };
        let uri = exec.request().expect("a request").uri().to_string();
        let expected = "/api/v1/namespaces/shop/pods/web-1/exec?&stdin=true&stdout=true&tty=true&container=app&command=mysql&command=-psecret";
        assert_eq!(uri, expected, "with a terminal, the error output comes on the standard output");
        assert_eq!(exec.what(), "an exec in pod shop/web-1 (container app)");
        let attach = Exec { command: None, tty: false, ..exec };
        assert!(
            attach
                .request()
                .expect("a request")
                .uri()
                .to_string()
                .contains("/attach?&stdin=true&stdout=true&stderr=true")
        );
        let error = Exec { pod: ObjectKey::new("", "web-1"), ..attach }.request().expect_err("no namespace");
        assert_eq!(error.to_string(), "a pod is in a namespace, and the exec has none");
    }
}
