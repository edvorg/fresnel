//! The answer of the server to one request: its status and its body.

use std::time::Duration;

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
    let body = response.into_body().collect().await.map_err(transport)?.to_bytes();
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
    let body = timeout(limit, capped(response.into_body())).await.ok().flatten();
    let status = body.and_then(|body| serde_json::from_slice::<Status>(&body).ok());
    match status.map(|status| status.message).filter(|message| !message.is_empty()) {
        Some(message) => CoreError::answered(code, format!("{message} (HTTP {code})")),
        None => CoreError::answered(code, format!("the server answered HTTP {code}")),
    }
}

/// Reads a body of `MAX_REFUSAL_BYTES` or fewer. `None` for a larger body and for one that breaks.
async fn capped(mut body: Body) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame.ok()?.into_data() {
            bytes.extend_from_slice(&data);
        }
        if bytes.len() > MAX_REFUSAL_BYTES {
            return None;
        }
    }
    Some(bytes)
}
