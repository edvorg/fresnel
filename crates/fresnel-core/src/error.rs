//! The one error type of the domain layer.

use std::error::Error;
use std::fmt::{self, Display};
use std::sync::Arc;

use fresnel_net::{ErrorClass, NetError};

/// An error of the domain layer. The class selects the text and the action of the UI.
///
/// The text never holds a token, a key or the content of an object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreError {
    class: ErrorClass,
    status: Option<u16>,
    text: Arc<str>,
    /// The causes of the `Status` of the server, as (reason, message): a step reads them,
    /// for example the PodDisruptionBudget that blocks an eviction.
    causes: Option<Arc<[(String, String)]>>,
}

impl CoreError {
    /// An error of this class. The text must hold no token, no key and no content of an object.
    pub fn new(class: ErrorClass, text: impl Into<Arc<str>>) -> Self {
        Self { class, status: None, text: text.into(), causes: None }
    }

    /// An entry or a request that cannot work as it is.
    pub(crate) fn invalid(text: impl Into<Arc<str>>) -> Self {
        Self::new(ErrorClass::Invalid, text)
    }

    /// The server answered with an error status. The class follows from the status.
    pub(crate) fn answered(status: u16, text: impl Into<Arc<str>>) -> Self {
        Self { class: class_of(status), status: Some(status), text: text.into(), causes: None }
    }

    /// The same error with the causes of the `Status` of the server: (reason, message).
    pub(crate) fn with_causes(self, causes: Vec<(String, String)>) -> Self {
        Self { causes: Some(causes).filter(|causes| !causes.is_empty()).map(Into::into), ..self }
    }

    /// The message of the first cause with this reason, for example `DisruptionBudget`.
    pub(crate) fn cause(&self, reason: &str) -> Option<&str> {
        let causes = self.causes.as_deref().unwrap_or_default();
        causes.iter().find(|(of, _)| of == reason).map(|(_, message)| message.as_str())
    }

    /// The class of the error.
    pub fn class(&self) -> ErrorClass {
        self.class
    }

    /// The HTTP status, when the error is an answer of the server. A caller reads it to
    /// tell an object that is gone (404) from a request that the server refuses.
    pub fn status(&self) -> Option<u16> {
        self.status
    }
}

/// The class of an HTTP status that is an error.
fn class_of(status: u16) -> ErrorClass {
    match status {
        401 => ErrorClass::Auth,
        403 => ErrorClass::Forbidden,
        409 => ErrorClass::Conflict,
        408 | 504 => ErrorClass::Timeout,
        429 | 500..=599 => ErrorClass::Server,
        _ => ErrorClass::Invalid,
    }
}

impl Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl Error for CoreError {}

impl From<NetError> for CoreError {
    fn from(error: NetError) -> Self {
        Self::new(error.class(), error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_class_of_an_answer_follows_its_status() {
        let class = |status| CoreError::answered(status, "refused").class();
        assert_eq!(class(401), ErrorClass::Auth);
        assert_eq!(class(403), ErrorClass::Forbidden);
        assert_eq!(class(409), ErrorClass::Conflict);
        assert_eq!((class(408), class(504)), (ErrorClass::Timeout, ErrorClass::Timeout));
        assert_eq!((class(429), class(500), class(503)), (ErrorClass::Server, ErrorClass::Server, ErrorClass::Server));
        assert_eq!(
            (class(400), class(404), class(422)),
            (ErrorClass::Invalid, ErrorClass::Invalid, ErrorClass::Invalid)
        );
        assert_eq!(CoreError::answered(404, "no such pod").status(), Some(404));
        assert_eq!(CoreError::invalid("no server").status(), None);
        let causes = vec![("DisruptionBudget".to_owned(), "The disruption budget web needs 1".to_owned())];
        let blocked = CoreError::answered(429, "Cannot evict pod").with_causes(causes);
        assert_eq!(blocked.cause("DisruptionBudget"), Some("The disruption budget web needs 1"));
        assert_eq!(
            (blocked.cause("NamespaceTerminating"), CoreError::invalid("x").cause("DisruptionBudget")),
            (None, None)
        );
    }
}
