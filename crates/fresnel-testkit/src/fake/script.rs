//! The scripted faults of the fake server: which requests a fault takes, and what they get.

use std::time::Duration;

use tokio::time::Instant;

use super::seen::SeenRequest;

/// One scripted fault. With no condition, it takes each request.
///
/// ```
/// use std::time::Duration;
/// use fresnel_testkit::Fault;
///
/// // The metrics API refuses the token for 2 seconds. Each other path works.
/// let fault = Fault::unauthorized().on_path("/apis/metrics.k8s.io/").for_time(Duration::from_secs(2));
/// ```
#[derive(Clone, Debug)]
pub struct Fault {
    pub(super) answer: FaultAnswer,
    path: Option<String>,
    token: Option<String>,
    until: Option<Instant>,
}

/// What a request gets in place of its normal answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FaultAnswer {
    Nothing,
    Unauthorized,
    Forbidden,
    NotFound,
    Unavailable,
    TooManyRequests { retry_after_seconds: u64 },
}

impl Fault {
    /// The request gets no answer. Its stream stays open until the client ends it.
    pub fn no_answer() -> Self {
        Self::new(FaultAnswer::Nothing)
    }

    /// The request gets HTTP 401, as from a server that refuses the token.
    pub fn unauthorized() -> Self {
        Self::new(FaultAnswer::Unauthorized)
    }

    /// The request gets HTTP 403, as from a server whose access rules do not permit it.
    pub fn forbidden() -> Self {
        Self::new(FaultAnswer::Forbidden)
    }

    /// The request gets HTTP 404, as from a server that does not serve the kind of the path.
    pub fn not_found() -> Self {
        Self::new(FaultAnswer::NotFound)
    }

    /// The request gets HTTP 503 with the text of the aggregator of an API server: an
    /// aggregated API whose backend does not answer, as a metrics-server that is down.
    pub fn unavailable() -> Self {
        Self::new(FaultAnswer::Unavailable)
    }

    /// The request gets HTTP 429 with the header `Retry-After` in full seconds.
    pub fn too_many_requests(retry_after: Duration) -> Self {
        Self::new(FaultAnswer::TooManyRequests { retry_after_seconds: retry_after.as_secs() })
    }

    fn new(answer: FaultAnswer) -> Self {
        Self { answer, path: None, token: None, until: None }
    }

    /// Only requests whose path starts with `prefix`.
    #[must_use]
    pub fn on_path(mut self, prefix: impl Into<String>) -> Self {
        self.path = Some(prefix.into());
        self
    }

    /// Only requests with this bearer token. A client with a new token then gets its answer.
    #[must_use]
    pub fn for_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Only for this time, from this call. Then the fault ends by itself.
    #[must_use]
    pub fn for_time(mut self, time: Duration) -> Self {
        self.until = Some(Instant::now() + time);
        self
    }

    /// True for an HTTP 401 of the server itself: its token check refuses the request. A
    /// fault on a path stands for an aggregated API, which refuses after that check passed.
    pub(super) fn refuses_the_credential(&self) -> bool {
        self.answer == FaultAnswer::Unauthorized && self.path.is_none()
    }

    pub(super) fn takes(&self, request: &SeenRequest, now: Instant) -> bool {
        self.path.as_ref().is_none_or(|prefix| request.uri.path().starts_with(prefix.as_str()))
            && self.token.as_ref().is_none_or(|token| request.bearer() == Some(token.as_str()))
            && self.until.is_none_or(|until| now < until)
    }
}
