//! The channels of the WebSocket protocols of the API server (`wsstream` of apimachinery):
//! the first byte of each binary message is the number of its channel.

use bytes::{BufMut as _, Bytes, BytesMut};
use fresnel_net::Writer;

use crate::error::CoreError;

/// The subprotocol with the close of one channel, from Kubernetes 1.30 (KEP-4006).
pub(super) const V5: &str = "v5.channel.k8s.io";
/// The subprotocol before it, with a `Status` object on the error channel. The port forward
/// over a WebSocket has only this one.
pub(super) const V4: &str = "v4.channel.k8s.io";

/// The standard input of an exec, and the data of the first port of a port forward.
pub(super) const STDIN: u8 = 0;
pub(super) const STDOUT: u8 = 1;
pub(super) const STDERR: u8 = 2;
/// The status of the process of an exec: a JSON `Status` object, its last message.
pub(super) const ERROR: u8 = 3;
/// The size of the terminal of an exec, as JSON.
pub(super) const RESIZE: u8 = 4;
/// v5: the close of the channel whose number is the next byte.
pub(super) const CLOSE: u8 = 255;

/// The largest piece of data in one message. The API server and the node read pieces of
/// 32 KiB, and a larger message of the app would only wait longer in the queue.
pub(super) const PIECE: usize = 32 * 1024;

/// One message on one channel.
pub(super) fn message(channel: u8, data: &[u8]) -> Bytes {
    let mut message = BytesMut::with_capacity(1 + data.len());
    message.put_u8(channel);
    message.extend_from_slice(data);
    message.freeze()
}

/// The channel and the data of a message of the server. A message with no byte has no channel.
pub(super) fn split(message: &Bytes) -> Option<(u8, Bytes)> {
    let channel = *message.first()?;
    Some((channel, message.slice(1..)))
}

/// Sends data on one channel in pieces of `PIECE` bytes at most. Each piece waits for room
/// in the queue of the socket, so a peer that reads slowly slows the caller and loses nothing.
pub(super) async fn send(writer: &Writer, channel: u8, data: &[u8]) -> Result<(), CoreError> {
    for piece in data.chunks(PIECE) {
        writer.send(message(channel, piece)).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_starts_with_its_channel_and_the_split_gives_it_back() {
        let sent = message(RESIZE, br#"{"Width":80,"Height":24}"#);
        assert_eq!(&sent[..], b"\x04{\"Width\":80,\"Height\":24}");
        assert_eq!(split(&sent), Some((RESIZE, Bytes::from_static(br#"{"Width":80,"Height":24}"#))));
        assert_eq!(split(&Bytes::from_static(b"\x01")), Some((STDOUT, Bytes::new())), "the open of a channel");
        assert_eq!(split(&Bytes::new()), None);
    }
}
