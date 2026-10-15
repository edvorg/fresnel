//! Where a source reads its streams from. [`Opener`] for a gateway sends the request on the
//! log channel of the cluster; a test of a source gives an opener with a script.

use bytes::Bytes;

use crate::cluster::{Gateway, LogStream};
use crate::error::CoreError;
use crate::logs::request::{self, Origin};
use crate::store::ObjectKey;

/// Opens the streams of the sources of a view.
pub(in crate::logs) trait Opener: Clone + Send + Sync + 'static {
    /// The data of one stream.
    type Stream: Chunks;

    /// Opens the stream of the log of a container. The result comes after the good status
    /// of the server.
    fn open(
        &self,
        pod: &ObjectKey,
        container: &str,
        previous: bool,
        origin: Origin,
    ) -> impl Future<Output = Result<Self::Stream, CoreError>> + Send;
}

/// The data of one stream, in the pieces that arrive.
pub(in crate::logs) trait Chunks: Send + 'static {
    /// The next piece. `None` at the end of the log.
    fn next(&mut self) -> impl Future<Output = Option<Result<Bytes, CoreError>>> + Send;
}

impl Opener for Gateway {
    type Stream = LogStream;

    async fn open(
        &self,
        pod: &ObjectKey,
        container: &str,
        previous: bool,
        origin: Origin,
    ) -> Result<LogStream, CoreError> {
        self.log_stream(request::stream(pod, container, previous, origin)?).await
    }
}

impl Chunks for LogStream {
    fn next(&mut self) -> impl Future<Output = Option<Result<Bytes, CoreError>>> + Send {
        LogStream::next(self)
    }
}
