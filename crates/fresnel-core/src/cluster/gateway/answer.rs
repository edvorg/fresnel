//! The answer of the server to one request: its status and its body.

use std::time::Duration;

use bytes::Bytes;
use fresnel_net::{ErrorClass, NetError};
use http::Response;
use http_body_util::BodyExt as _;
use kube::client::Body;
use kube::core::Status;
use serde::de::DeserializeOwned;
use tokio::time::timeout;

use crate::error::CoreError;

/// A `Status` object of the server is small. A larger error body is not read to its end.
const MAX_REFUSAL_BYTES: usize = 1024 * 1024;

/// The most bytes of one answer that the gateway holds in memory. A list of 10,000 pods in
/// one page has about 300 MiB. A longer answer is a broken server, and it must not use all
/// the memory of the app.
const MAX_ANSWER_BYTES: usize = 1024 * 1024 * 1024;

/// The error of a request that got no answer, as it comes out of the kube client. An error
/// of the connection layer keeps its class.
pub(super) fn transport(error: kube::Error) -> CoreError {
    let other = match error {
        kube::Error::Service(cause) => match cause.downcast::<NetError>() {
            Ok(net) => return CoreError::from(*net),
            Err(other) => other.to_string(),
        },
        other => other.to_string(),
    };
    CoreError::new(ErrorClass::Network, format!("the request failed: {other}"))
}

/// The body of an answer with a good status, as JSON of the type `T`.
pub(super) async fn json<T: DeserializeOwned>(response: Response<Body>) -> Result<T, CoreError> {
    let body = read(response.into_body(), MAX_ANSWER_BYTES).await?;
    serde_json::from_slice(&body).map_err(|error| not_json::<T>(&error))
}

/// The error for JSON that does not fit its type. It gives the position only: the text of
/// the JSON reader can hold a value of the object, for example of a Secret.
pub(super) fn not_json<T>(error: &serde_json::Error) -> CoreError {
    let (kind, line, column) = (std::any::type_name::<T>(), error.line(), error.column());
    CoreError::invalid(format!(
        "the answer of the server is not a `{kind}`: JSON error at line {line}, column {column}"
    ))
}

/// The error for an answer with an error status. Its text is the message of the `Status`
/// object of the server. `limit` is for the read of that object: the body of a stream
/// request has no time limit of its own.
pub(super) async fn refusal(response: Response<Body>, limit: Duration) -> CoreError {
    let code = response.status().as_u16();
    // A body that is too long, too slow or broken gives no message.
    let body = timeout(limit, read(response.into_body(), MAX_REFUSAL_BYTES)).await.ok().and_then(Result::ok);
    let status = body.and_then(|body| serde_json::from_slice::<Status>(&body).ok());
    match status.map(|status| status.message).filter(|message| !message.is_empty()) {
        Some(message) => CoreError::answered(code, format!("{message} (HTTP {code})")),
        None => CoreError::answered(code, format!("the server answered HTTP {code}")),
    }
}

/// Reads a body to its end. A body of more than `max` bytes is an error, and the read ends
/// there.
async fn read<B>(mut body: B, max: usize) -> Result<Vec<u8>, CoreError>
where
    B: http_body::Body<Data = Bytes, Error = kube::Error> + Unpin,
{
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame.map_err(transport)?.into_data() {
            bytes.extend_from_slice(&data);
        }
        if bytes.len() > max {
            return Err(CoreError::invalid(format!("the answer of the server has more than {max} bytes")));
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use futures_util::stream;
    use http_body::Frame;
    use http_body_util::StreamBody;

    use super::*;

    /// A body that arrives in these pieces. `None` is a broken connection.
    fn body(pieces: &[Option<&'static str>]) -> impl http_body::Body<Data = Bytes, Error = kube::Error> + Unpin {
        let frame = |piece: &Option<&'static str>| match piece {
            Some(text) => Ok(Frame::data(Bytes::from_static(text.as_bytes()))),
            None => Err(kube::Error::Service("the connection broke".into())),
        };
        StreamBody::new(stream::iter(pieces.iter().map(frame).collect::<Vec<_>>()))
    }

    #[tokio::test]
    async fn a_read_gives_each_byte_and_stops_at_the_limit_and_at_a_broken_body() {
        let full = read(body(&[Some("{\"a\":"), Some("1}")]), 7).await.expect("seven bytes fit");
        assert_eq!(full, b"{\"a\":1}");

        let error = read(body(&[Some("{\"a\":"), Some("1}"), Some("never read")]), 6).await.expect_err("too long");
        let text = "the answer of the server has more than 6 bytes";
        assert_eq!((error.class(), error.to_string().as_str()), (ErrorClass::Invalid, text));

        // A cut answer is no answer: the bytes before the cut are not a result.
        let error = read(body(&[Some("{\"a\":1}"), None]), 100).await.expect_err("the body broke");
        let text = "the request failed: the connection broke";
        assert_eq!((error.class(), error.to_string().as_str()), (ErrorClass::Network, text));
    }
}
