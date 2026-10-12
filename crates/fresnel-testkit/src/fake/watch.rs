//! The watch streams of the fake server. A test sends each event line.

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::stream;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use tokio::sync::broadcast;

use super::answer::Body;
use super::state::Shared;

/// The events that one watch stream can be behind. A stream that is more behind ends, and
/// its client must start a new watch, as after a close by a real server.
pub(super) const FEED_EVENTS: usize = 4096;

/// What a test sends to the open watch streams.
#[derive(Clone)]
pub(super) enum Feed {
    /// One line for the streams of one path.
    Event { path: Arc<str>, line: Bytes },
    /// The clean end of each stream, as the server timeout of a watch.
    End,
}

/// One open watch stream. It counts as open until its body drops.
struct Watch {
    feed: broadcast::Receiver<Feed>,
    path: Arc<str>,
    shared: Arc<Shared>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.shared.count(|counters| counters.open_watches = counters.open_watches.saturating_sub(1));
    }
}

/// The body of a watch response. The stream is open for events before this function returns,
/// so a test that waits for the counter loses no event.
pub(super) fn body(shared: &Arc<Shared>, path: &str) -> Body {
    let watch = Watch { feed: shared.feed.subscribe(), path: path.into(), shared: Arc::clone(shared) };
    shared.count(|counters| counters.open_watches += 1);
    let lines = stream::unfold(watch, |mut watch| async move {
        loop {
            match watch.feed.recv().await {
                Ok(Feed::Event { path, line }) if path == watch.path => {
                    return Some((Ok::<_, Infallible>(Frame::data(line)), watch));
                }
                Ok(Feed::Event { .. }) => {}
                Ok(Feed::End) | Err(_) => return None,
            }
        }
    });
    StreamBody::new(lines).boxed_unsync()
}
