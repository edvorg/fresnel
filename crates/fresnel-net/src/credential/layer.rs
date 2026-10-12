//! The token of a request, and the rule for an HTTP 401 answer.
//!
//! After an HTTP 401 the request gets a new token and goes out one more time. An HTTP 401
//! means that the server did not run the request, so the second send is also safe for a
//! write. After a second HTTP 401 the credential task proves the token. A proof that passes
//! gives the answer of the server to the caller, and no other request sees a change.

use bytes::Bytes;
use http::header::AUTHORIZATION;
use http::{Request, Response, StatusCode};

use super::Credentials;
use super::state::Issued;
use crate::error::NetError;

/// Sends the request with the token of the cluster. `next` sends one request to the server.
pub(crate) async fn send<R, F>(
    credentials: &Credentials,
    request: Request<Bytes>,
    next: impl Fn(Request<Bytes>) -> F,
) -> Result<Response<R>, NetError>
where
    F: Future<Output = Result<Response<R>, NetError>>,
{
    let token = credentials.token().await?;
    let first = next(with_token(copy_of(&request), &token)).await?;
    if first.status() != StatusCode::UNAUTHORIZED {
        return Ok(first);
    }
    let Some(newer) = credentials.after_refusal(&token).await? else {
        return Ok(first);
    };
    // The drop ends the stream of the first answer before the second send.
    drop(first);
    let second = next(with_token(request, &newer)).await?;
    if second.status() == StatusCode::UNAUTHORIZED {
        // The task decides now. Only a credential that failed changes the result; in each
        // other case the HTTP 401 of the server goes to the caller, and no third send follows.
        credentials.after_refusal(&newer).await?;
    }
    Ok(second)
}

fn with_token(mut request: Request<Bytes>, token: &Issued) -> Request<Bytes> {
    request.headers_mut().insert(AUTHORIZATION, token.header.clone());
    request
}

/// A second request with the same content.
pub(crate) fn copy_of(request: &Request<Bytes>) -> Request<Bytes> {
    let mut copy = Request::new(request.body().clone());
    *copy.method_mut() = request.method().clone();
    *copy.uri_mut() = request.uri().clone();
    *copy.version_mut() = request.version();
    *copy.headers_mut() = request.headers().clone();
    *copy.extensions_mut() = request.extensions().clone();
    copy
}
