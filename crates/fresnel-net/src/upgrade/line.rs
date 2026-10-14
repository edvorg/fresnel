//! One HTTP/1.1 connection of the upgrade client.

use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use futures_util::FutureExt as _;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt as _, Full};
use hyper::client::conn::http1::{self, SendRequest};
use hyper_util::rt::TokioIo;
use tokio::time::timeout;

use crate::body::ResponseBody;
use crate::connect::dial::Protocol;
use crate::connect::{Dial, Effort};
use crate::error::NetError;

/// One HTTP/1.1 connection with no request in progress.
///
/// hyper needs a poll of `connection` for each read and write. A line has no task of its
/// own: the code that uses the line polls it.
pub(super) struct Line {
    sender: SendRequest<Full<Bytes>>,
    connection: Pin<Box<dyn Future<Output = hyper::Result<()>> + Send>>,
}

impl Line {
    /// Opens a new connection with the hedged connect of the cluster.
    pub(super) async fn open(dial: &dyn Dial) -> Result<Self, NetError> {
        let io = dial.dial(Protocol::Http1, Effort::Race).await?;
        let (sender, connection) = http1::handshake(TokioIo::new(io))
            .await
            .map_err(|error| NetError::because("the HTTP/1.1 handshake failed", &error))?;
        Ok(Self { sender, connection: Box::pin(connection.with_upgrades()) })
    }

    /// Sends a light request and reads its full answer. After `Ok`, the connection worked a
    /// moment before and is idle again.
    pub(super) async fn test(&mut self, request: Request<Full<Bytes>>, limit: Duration) -> Result<(), NetError> {
        let exchange = async {
            let mut body = self.sender.send_request(request).await?.into_body();
            while let Some(frame) = body.frame().await {
                frame?;
            }
            Ok::<_, hyper::Error>(())
        };
        tokio::select! {
            answered = timeout(limit, exchange) => match answered {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(NetError::because("the test of a warm connection failed", &error)),
                Err(_) => Err(NetError::timeout(format!("a warm connection gave no answer in {limit:?}"))),
            },
            _ended = &mut self.connection => Err(NetError::network("a warm connection closed")),
        }
    }

    /// Sends the request of a caller. The line ends with this request: an upgrade takes
    /// its connection, and each other answer is the last one on it.
    pub(super) async fn send(self, request: Request<Full<Bytes>>) -> Result<Response<ResponseBody>, NetError> {
        let Self { mut sender, mut connection } = self;
        let broke = |error: hyper::Error| NetError::because("the upgrade request failed", &error);
        let answer = sender.send_request(request);
        tokio::pin!(answer);
        let mut handed_over = false;
        let response = loop {
            tokio::select! {
                response = &mut answer => break response.map_err(broke)?,
                ended = &mut connection, if !handed_over => {
                    ended.map_err(broke)?;
                    handed_over = true;
                }
            }
        };
        if response.status() == StatusCode::SWITCHING_PROTOCOLS {
            // hyper gives the raw stream to the caller only after the end of the connection future.
            if !handed_over {
                connection.await.map_err(broke)?;
            }
            return Ok(response.map(ResponseBody::new));
        }
        // An answer with a body, for example HTTP 403: the reader of the body runs the
        // connection. The end of the connection tells the reader nothing new: hyper gives an
        // error of the connection to the body too.
        let connection = async move {
            match connection.await {
                Ok(()) | Err(_) => {}
            }
        }
        .boxed();
        Ok(response.map(|body| ResponseBody::new(body).with_connection(connection)))
    }
}
