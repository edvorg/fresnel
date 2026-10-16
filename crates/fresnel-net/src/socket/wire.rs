//! The wire of a socket: the write of its queue, the delivery to its owner, its messages and
//! the end that a read error or a close message of the server gives.

use std::collections::VecDeque;
use std::task::{Poll, ready};

use bytes::Bytes;
use futures_util::SinkExt as _;
use futures_util::stream::SplitSink;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use super::{SocketEnd, SocketFailure};
use crate::error::chain;

pub(super) type Sink<S> = SplitSink<WebSocketStream<S>, Message>;

/// Hands each message of the queue to the sink and writes each byte out. It keeps no state
/// of its own: a message leaves the queue only into the sink, so the end of a wait in the
/// middle loses nothing.
pub(super) fn write_out<'a, S>(
    sink: &'a mut Sink<S>,
    queue: &'a mut VecDeque<Message>,
    unflushed: &'a mut bool,
) -> impl Future<Output = Result<(), WsError>> + 'a
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    std::future::poll_fn(move |cx| {
        while !queue.is_empty() {
            ready!(sink.poll_ready_unpin(cx))?;
            if let Some(message) = queue.pop_front() {
                sink.start_send_unpin(message)?;
                *unflushed = true;
            }
        }
        ready!(sink.poll_flush_unpin(cx))?;
        *unflushed = false;
        Poll::Ready(Ok(()))
    })
}

/// Gives the held message to the owner when its queue has room. With no receiver, the owner
/// reads no more and drops the socket soon, so the message goes.
pub(super) async fn deliver(delivery: &mpsc::Sender<Bytes>, held: &mut Option<Bytes>) {
    let permit = delivery.reserve().await;
    if let (Ok(permit), Some(message)) = (permit, held.take()) {
        permit.send(message);
    }
    *held = None;
}

pub(super) async fn at(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// A ping with the count of the data bytes before it, which its answer gives back.
pub(super) fn ping(sent: u64) -> Message {
    Message::Ping(Bytes::copy_from_slice(&sent.to_be_bytes()))
}

/// The count in the answer to a ping. A payload of an other form confirms nothing.
pub(super) fn count(payload: &[u8]) -> u64 {
    <[u8; 8]>::try_from(payload).map(u64::from_be_bytes).unwrap_or_default()
}

/// The close message of the app: a normal close.
pub(super) fn close() -> Message {
    Message::Close(Some(CloseFrame { code: CloseCode::Normal, reason: "".into() }))
}

/// The end of a close message of the server. A message with no code has the code 1005.
pub(super) fn closed_by_server(frame: Option<CloseFrame>) -> SocketEnd {
    let (code, reason) = frame.map_or((1005, String::new()), |frame| (frame.code.into(), frame.reason.to_string()));
    SocketEnd::ClosedByServer { code, reason }
}

/// The end of a read that failed.
pub(super) fn read_failed(error: WsError) -> SocketEnd {
    match error {
        // The end of the connection with no close message.
        WsError::ConnectionClosed
        | WsError::AlreadyClosed
        | WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => SocketEnd::EndedByServer,
        WsError::Io(error) => SocketEnd::Failed(SocketFailure::Broken(chain(&error))),
        WsError::Protocol(_) | WsError::Capacity(_) | WsError::Utf8(_) | WsError::AttackAttempt => {
            SocketEnd::Failed(SocketFailure::Invalid(error.to_string()))
        }
        other => SocketEnd::Failed(SocketFailure::Broken(other.to_string())),
    }
}
