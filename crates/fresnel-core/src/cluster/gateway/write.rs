//! What the app knows about a write that failed (design rule 6).
//!
//! The gateway sends a write one time. The fate of a failed write tells the caller what it
//! can do next: send again, show the refusal, or read the object.

use std::error::Error;
use std::fmt::{self, Display};

use fresnel_net::ErrorClass;

use crate::error::CoreError;

/// What happened to a write that gave no result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fate {
    /// The request did not go to the server. One more send is safe.
    NotSent,
    /// The server refused the write, and nothing changed.
    Refused,
    /// The request went out, and no good answer came back. The write can be in effect.
    /// The app must read the object, and it must not send the write again.
    Unknown,
}

/// A write that gave no result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteError {
    /// What happened to the write.
    pub fate: Fate,
    /// Why.
    pub error: CoreError,
}

impl WriteError {
    /// The gateway did not send the request.
    pub(super) fn not_sent(error: CoreError) -> Self {
        Self { fate: Fate::NotSent, error }
    }

    /// The fate of a write whose request gave `error` in place of a good status.
    ///
    /// The rule is on the safe side. The connection layer does not tell if a request left
    /// before a network error or a timeout, so those count as sent.
    pub(super) fn of(error: CoreError) -> Self {
        let fate = match (error.status(), error.class()) {
            // The server can fail after it stored the object, for example at a timeout of
            // its own database.
            (Some(500..), _) => Fate::Unknown,
            (Some(_), _) => Fate::Refused,
            // No token, a request that cannot be sent, or the throttle of the server: each
            // of these errors comes before the send.
            (None, ErrorClass::Auth | ErrorClass::Invalid | ErrorClass::Server) => Fate::NotSent,
            (None, _) => Fate::Unknown,
        };
        Self { fate, error }
    }

    /// The server accepted the write, and its answer broke or is not the expected object.
    pub(super) fn after_acceptance(error: CoreError) -> Self {
        Self { fate: Fate::Unknown, error }
    }
}

impl Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fate = match self.fate {
            Fate::NotSent => "the write was not sent",
            Fate::Refused => "the server refused the write",
            Fate::Unknown => "the result of the write is not known",
        };
        write!(f, "{fate}: {}", self.error)
    }
}

impl Error for WriteError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fate_of_a_write_follows_the_error_on_the_safe_side() {
        let answered = |status| WriteError::of(CoreError::answered(status, "refused")).fate;
        assert_eq!(
            (answered(403), answered(404), answered(409), answered(422)),
            (Fate::Refused, Fate::Refused, Fate::Refused, Fate::Refused)
        );
        assert_eq!(answered(429), Fate::Refused, "the server did not run a request that it throttled");
        assert_eq!((answered(500), answered(503), answered(504)), (Fate::Unknown, Fate::Unknown, Fate::Unknown));

        let no_answer = |class| WriteError::of(CoreError::new(class, "no answer")).fate;
        assert_eq!((no_answer(ErrorClass::Auth), no_answer(ErrorClass::Invalid)), (Fate::NotSent, Fate::NotSent));
        assert_eq!(no_answer(ErrorClass::Server), Fate::NotSent, "the throttle of the connection layer");
        assert_eq!((no_answer(ErrorClass::Network), no_answer(ErrorClass::Timeout)), (Fate::Unknown, Fate::Unknown));

        let error = WriteError::of(CoreError::answered(409, "the object changed (HTTP 409)"));
        assert_eq!(error.to_string(), "the server refused the write: the object changed (HTTP 409)");
    }
}
