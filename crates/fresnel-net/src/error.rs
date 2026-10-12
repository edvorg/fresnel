//! The one error type of the connection layer.

use std::error::Error;
use std::fmt::{self, Display};
use std::sync::Arc;

/// The class of an error. The UI selects its text and its action from the class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorClass {
    /// No valid credential: the plugin failed, or the server refused the token.
    Auth,
    /// The server does not permit the action.
    Forbidden,
    /// The object changed on the server before the write.
    Conflict,
    /// No result in the time limit.
    Timeout,
    /// No connection, or the connection broke.
    Network,
    /// The server answered with an error of its own, for example a throttle.
    Server,
    /// The request cannot be sent as it is.
    Invalid,
}

/// An error of the connection layer.
///
/// One failed connect goes to each request that waited for it, so a copy is cheap. The text
/// never holds a token and never holds text that a child process wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetError {
    class: ErrorClass,
    text: Arc<str>,
}

impl NetError {
    pub(crate) fn new(class: ErrorClass, text: impl Into<Arc<str>>) -> Self {
        Self { class, text: text.into() }
    }

    pub(crate) fn network(text: impl Into<Arc<str>>) -> Self {
        Self::new(ErrorClass::Network, text)
    }

    pub(crate) fn timeout(text: impl Into<Arc<str>>) -> Self {
        Self::new(ErrorClass::Timeout, text)
    }

    pub(crate) fn invalid(text: impl Into<Arc<str>>) -> Self {
        Self::new(ErrorClass::Invalid, text)
    }

    /// A network error with the step that failed before the text of its cause.
    pub(crate) fn because(step: impl Display, cause: &(dyn Error + 'static)) -> Self {
        Self::network(format!("{step}: {}", chain(cause)))
    }

    /// The class of the error.
    pub fn class(&self) -> ErrorClass {
        self.class
    }
}

impl Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl Error for NetError {}

/// The text of an error with the text of each source, so that no cause is lost.
pub(crate) fn chain(error: &(dyn Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        // Some errors repeat the text of their source.
        if !text.ends_with(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    #[derive(Debug)]
    struct Outer(io::Error);

    impl Display for Outer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("the handshake failed")
        }
    }

    impl Error for Outer {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn the_text_keeps_each_cause_one_time() {
        let error = NetError::because("connect", &Outer(io::Error::other("reset by peer")));
        assert_eq!(error.to_string(), "connect: the handshake failed: reset by peer");
        assert_eq!(error.class(), ErrorClass::Network);
        assert_eq!(chain(&io::Error::other("alone")), "alone");
    }
}
