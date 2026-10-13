//! The one error type of the test kit.

use std::fmt::{self, Display};
use std::io;

/// An error of the test kit. A test stops at it, thus it holds only the text of the cause.
#[derive(Debug)]
pub struct KitError {
    text: String,
    /// True when the process of k3s ended by itself.
    k3s_stopped: bool,
}

impl KitError {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), k3s_stopped: false }
    }

    /// An error with the step that failed before its cause.
    pub(crate) fn because(step: impl Display, cause: impl Display) -> Self {
        Self::new(format!("{step}: {cause}"))
    }

    /// The process of k3s ended by itself. The text has its last lines.
    pub(crate) fn k3s_stopped(text: impl Into<String>) -> Self {
        Self { text: text.into(), k3s_stopped: true }
    }

    pub(crate) fn is_k3s_stopped(&self) -> bool {
        self.k3s_stopped
    }
}

impl Display for KitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for KitError {}

impl From<io::Error> for KitError {
    fn from(error: io::Error) -> Self {
        Self::new(error.to_string())
    }
}
