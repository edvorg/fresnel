//! The one error type of the test kit.

use std::fmt::{self, Display};
use std::io;

/// An error of the test kit. A test stops at it, thus it holds only the text of the cause.
#[derive(Debug)]
pub struct KitError(String);

impl KitError {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// An error with the step that failed before its cause.
    pub(crate) fn because(step: impl Display, cause: impl Display) -> Self {
        Self(format!("{step}: {cause}"))
    }
}

impl Display for KitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KitError {}

impl From<io::Error> for KitError {
    fn from(error: io::Error) -> Self {
        Self(error.to_string())
    }
}
