//! The streams of the gateway: the events of a watch and the data of a log.
//!
//! A stream has no time limit on its progress (design rule 2). The watch loop and the log
//! view decide what a stream with no data means.

use std::marker::PhantomData;

use bytes::{Bytes, BytesMut};
use http_body_util::BodyExt as _;
use http_body_util::combinators::UnsyncBoxBody;
use kube::core::WatchEvent;
use serde::de::DeserializeOwned;

use super::answer;
use crate::error::CoreError;

/// The body of a stream answer, with the error of the gateway.
type Data = UnsyncBoxBody<Bytes, CoreError>;

fn data_of(body: kube::client::Body) -> Data {
    body.map_err(answer::transport).boxed_unsync()
}

/// One event line holds one object, and an object of the API has some megabytes at most.
/// A longer line is a broken stream.
const MAX_EVENT_BYTES: usize = 32 * 1024 * 1024;

/// The events of one watch.
pub struct WatchStream<K> {
    /// `None` after the end or an error of the stream, so that its connection stream is
    /// free at once.
    body: Option<Data>,
    buffer: BytesMut,
    /// The count of bytes at the start of the buffer that hold no line end.
    scanned: usize,
    kind: PhantomData<fn() -> K>,
}

impl<K: DeserializeOwned> WatchStream<K> {
    pub(super) fn new(body: kube::client::Body) -> Self {
        Self::over(data_of(body))
    }

    fn over(body: Data) -> Self {
        Self { body: Some(body), buffer: BytesMut::new(), scanned: 0, kind: PhantomData }
    }

    /// The next event. `None` is the end of the stream: the server ends each watch after
    /// its timeout. An `ERROR` event of the server, for example for an old resource version,
    /// is an event and not an `Err`.
    pub async fn next(&mut self) -> Option<Result<WatchEvent<K>, CoreError>> {
        loop {
            if let Some(line) = self.take_line() {
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let event = serde_json::from_slice(&line);
                return Some(event.map_err(|error| answer::not_json::<WatchEvent<K>>(&error)));
            }
            match next_data(&mut self.body).await? {
                Ok(data) => self.buffer.extend_from_slice(&data),
                Err(error) => return Some(Err(error)),
            }
            if self.buffer.len() > MAX_EVENT_BYTES {
                self.body = None;
                let text = format!("one event of the watch has more than {MAX_EVENT_BYTES} bytes");
                return Some(Err(CoreError::invalid(text)));
            }
        }
    }

    /// The first full line of the buffer, with its line end.
    fn take_line(&mut self) -> Option<BytesMut> {
        let unread = self.buffer.get(self.scanned..)?;
        match unread.iter().position(|byte| *byte == b'\n') {
            Some(end) => {
                let line = self.buffer.split_to(self.scanned + end + 1);
                self.scanned = 0;
                Some(line)
            }
            None => {
                self.scanned = self.buffer.len();
                None
            }
        }
    }
}

impl<K> std::fmt::Debug for WatchStream<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchStream").field("ended", &self.body.is_none()).finish_non_exhaustive()
    }
}

/// The data of one log, in the pieces that arrive. A piece can end in the middle of a line.
pub struct LogStream {
    body: Option<Data>,
}

impl LogStream {
    pub(super) fn new(body: kube::client::Body) -> Self {
        Self { body: Some(data_of(body)) }
    }

    /// The next piece of data. `None` is the end of the log: the container ended, or the
    /// request did not ask to follow.
    pub async fn next(&mut self) -> Option<Result<Bytes, CoreError>> {
        next_data(&mut self.body).await
    }
}

impl std::fmt::Debug for LogStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogStream").field("ended", &self.body.is_none()).finish()
    }
}

/// The next data of a body. `None` at its end. The body drops at its end and at an error.
async fn next_data(body: &mut Option<Data>) -> Option<Result<Bytes, CoreError>> {
    loop {
        let frame = body.as_mut()?.frame().await;
        match frame {
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    return Some(Ok(data));
                }
            }
            Some(Err(error)) => {
                *body = None;
                return Some(Err(error));
            }
            None => {
                *body = None;
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use futures_util::stream;
    use http_body::Frame;
    use http_body_util::StreamBody;
    use serde_json::Value;

    use super::*;
    use crate::net::ErrorClass;

    /// A watch whose body arrives in these pieces. `None` is a broken connection.
    fn watch<K: DeserializeOwned>(pieces: &[Option<&'static str>]) -> WatchStream<K> {
        let frames: Vec<Result<Frame<Bytes>, CoreError>> = pieces
            .iter()
            .map(|piece| match piece {
                Some(text) => Ok(Frame::data(Bytes::from_static(text.as_bytes()))),
                None => Err(CoreError::new(ErrorClass::Network, "the connection broke")),
            })
            .collect();
        WatchStream::over(StreamBody::new(stream::iter(frames)).boxed_unsync())
    }

    fn name(event: Option<Result<WatchEvent<Value>, CoreError>>) -> String {
        match event.expect("an event").expect("a good event") {
            WatchEvent::Added(object) => format!("added {}", object["name"]),
            WatchEvent::Modified(object) => format!("modified {}", object["name"]),
            WatchEvent::Deleted(object) => format!("deleted {}", object["name"]),
            WatchEvent::Bookmark(bookmark) => format!("bookmark {}", bookmark.metadata.resource_version),
            WatchEvent::Error(status) => format!("error {}", status.code),
        }
    }

    #[tokio::test]
    async fn the_events_come_one_for_each_line_however_the_data_arrives() {
        let mut events = watch::<Value>(&[
            Some(r#"{"type":"ADDED","object":{"name":1}}"#),
            Some("\n"),
            Some(r#"{"type":"MODIFIED","object":{"name":2}}"#),
            Some("\n\n"),
            Some(r#"{"type":"DELE"#),
            Some(r#"TED","object":{"na"#),
            Some("me\":3}}\n{\"type\":\"BOOKMARK\",\"object\":{\"kind\":\"Pod\",\"apiVersion\":\"v1\","),
            Some("\"metadata\":{\"resourceVersion\":\"77\"}}}\n{\"type\":\"ERROR\",\"object\":{\"code\":410}}\n"),
            Some(r#"{"type":"ADDED","object":{"name":"cut"#),
        ]);
        for expected in ["added 1", "modified 2", "deleted 3", "bookmark 77", "error 410"] {
            assert_eq!(name(events.next().await), expected);
        }
        assert!(events.next().await.is_none(), "a line with no end is not an event");
        assert!(events.next().await.is_none(), "the stream stays ended");
    }

    #[tokio::test]
    async fn a_bad_line_is_an_error_with_no_text_of_the_object_and_the_stream_continues() {
        // The JSON reader gives the text of a value that does not fit its type.
        let mut numbers = watch::<u32>(&[Some("{\"type\":\"ADDED\",\"object\":\"secret-value\"}\n")]);
        let error = numbers.next().await.expect("an item").expect_err("the object is not a number");
        let expected =
            "the answer of the server is not a `kube_core::watch::WatchEvent<u32>`: JSON error at line 1, column 39";
        assert_eq!((error.class(), error.to_string().as_str()), (ErrorClass::Invalid, expected));

        let mut events = watch::<Value>(&[
            Some("{\"type\":\"ADDED\",\"object\":\n"),
            Some("{\"type\":\"ADDED\",\"object\":{\"name\":4}}\n"),
            None,
            Some("{\"type\":\"ADDED\",\"object\":{\"name\":5}}\n"),
        ]);
        let error = events.next().await.expect("an item").expect_err("the line is not an event");
        assert_eq!(error.class(), ErrorClass::Invalid, "{error}");
        assert_eq!(name(events.next().await), "added 4");

        let error = events.next().await.expect("an item").expect_err("the connection broke");
        assert_eq!((error.class(), error.to_string().as_str()), (ErrorClass::Network, "the connection broke"));
        assert!(events.next().await.is_none(), "no event after the error of the connection");
    }
}
