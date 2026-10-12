//! The body of a response.
//!
//! It holds its stream in the count of the channel until it drops, it applies the time
//! limits of its request, and it decodes gzip. The caller reads it as a normal body.

mod gzip;
mod limits;

use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::future::BoxFuture;
use http::header::{CONTENT_ENCODING, CONTENT_LENGTH};
use http::{HeaderMap, HeaderValue};
use http_body::{Body, Frame};
use http_body_util::BodyExt as _;
use http_body_util::combinators::UnsyncBoxBody;

use crate::channel::streams::StreamPermit;
use crate::error::NetError;
use gzip::Gunzip;
pub(crate) use limits::BodyLimits;

/// The body of a response of the connection layer.
pub struct ResponseBody {
    /// `None` after the end or an error, so that the stream of the body is free at once.
    source: Option<UnsyncBoxBody<Bytes, NetError>>,
    permit: Option<StreamPermit>,
    limits: Option<BodyLimits>,
    gunzip: Option<Gunzip>,
    /// The HTTP/1.1 connection of the body. Only the reader of the body runs it.
    connection: Option<BoxFuture<'static, ()>>,
}

impl ResponseBody {
    pub(crate) fn new<B>(source: B) -> Self
    where
        B: Body<Data = Bytes> + Send + 'static,
        B::Error: std::error::Error + 'static,
    {
        let source = source.map_err(|error| NetError::because("the answer broke", &error)).boxed_unsync();
        Self { source: Some(source), permit: None, limits: None, gunzip: None, connection: None }
    }

    /// The stream of the body counts as open until the body drops.
    pub(crate) fn on_stream(mut self, permit: StreamPermit) -> Self {
        self.permit = Some(permit);
        self
    }

    pub(crate) fn with_limits(mut self, limits: Option<BodyLimits>) -> Self {
        self.limits = limits;
        self
    }

    /// Decodes the body if the server sent gzip, and removes the headers of the coded form.
    pub(crate) fn decoded(mut self, headers: &mut HeaderMap) -> Self {
        if headers.get(CONTENT_ENCODING) == Some(&HeaderValue::from_static("gzip")) {
            headers.remove(CONTENT_ENCODING);
            headers.remove(CONTENT_LENGTH);
            self.gunzip = Some(Gunzip::new());
        }
        self
    }

    pub(crate) fn with_connection(mut self, connection: BoxFuture<'static, ()>) -> Self {
        self.connection = Some(connection);
        self
    }

    /// Ends the body. The stream and its place in the count are free at once.
    fn end(&mut self, last: Option<Result<Frame<Bytes>, NetError>>) -> Poll<Option<Result<Frame<Bytes>, NetError>>> {
        self.source = None;
        self.permit = None;
        self.connection = None;
        Poll::Ready(last)
    }

    /// One piece of data on its way to the reader. `None` when the decoder needs more data.
    fn data(&mut self, piece: Bytes) -> Option<Result<Bytes, NetError>> {
        let Some(gunzip) = &mut self.gunzip else {
            return Some(Ok(piece));
        };
        match gunzip.feed(&piece) {
            Ok(decoded) if decoded.is_empty() => None,
            Ok(decoded) => Some(Ok(decoded)),
            Err(error) => Some(Err(NetError::because("the gzip data of the answer is bad", &error))),
        }
    }

    /// The last piece of a gzip body, and the check that the gzip data is complete.
    fn last_data(&mut self) -> Option<Result<Frame<Bytes>, NetError>> {
        match self.gunzip.as_mut()?.finish() {
            Ok(rest) if rest.is_empty() => None,
            Ok(rest) => Some(Ok(Frame::data(rest))),
            Err(error) => Some(Err(NetError::because("the gzip data of the answer ended early", &error))),
        }
    }
}

impl Body for ResponseBody {
    type Data = Bytes;
    type Error = NetError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, NetError>>> {
        let this = self.get_mut();
        if let Some(connection) = &mut this.connection
            && connection.as_mut().poll(cx).is_ready()
        {
            this.connection = None;
        }
        loop {
            let Some(source) = &mut this.source else {
                return Poll::Ready(None);
            };
            match Pin::new(source).poll_frame(cx) {
                Poll::Ready(Some(Ok(frame))) => {
                    if let Some(permit) = &this.permit {
                        permit.frame_arrived();
                    }
                    if let Some(limits) = &mut this.limits {
                        limits.data_arrived();
                    }
                    match frame.into_data().map(|piece| this.data(piece)) {
                        Ok(Some(Ok(piece))) => return Poll::Ready(Some(Ok(Frame::data(piece)))),
                        Ok(Some(Err(error))) => return this.end(Some(Err(error))),
                        Ok(None) => {}
                        Err(trailers) => return Poll::Ready(Some(Ok(trailers))),
                    }
                }
                Poll::Ready(Some(Err(error))) => return this.end(Some(Err(error))),
                Poll::Ready(None) => {
                    let last = this.last_data();
                    return this.end(last);
                }
                Poll::Pending => {
                    return match this.limits.as_mut().and_then(|limits| limits.poll_passed(cx)) {
                        Some(error) => this.end(Some(Err(error))),
                        None => Poll::Pending,
                    };
                }
            }
        }
    }
}

impl fmt::Debug for ResponseBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResponseBody").field("ended", &self.source.is_none()).finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
