//! The error of one plugin run.
//!
//! The plugin writes its error text for the user. That text can hold an account name or a
//! path, so it shows in the UI only. `Display` and `Debug` of the error never have it, thus
//! no log line and no journal file can get it by mistake.

use std::fmt::{self, Display};
use std::time::Duration;

/// Why a plugin run gave no token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginFailure {
    /// The kubeconfig gives no command.
    NoCommand,
    /// The command is in none of the folders of `PATH`.
    NotFound {
        /// The count of folders in `PATH`.
        searched: usize,
    },
    /// The process did not start. The text is the error of the system.
    Start(String),
    /// The process did not end in the time limit.
    Timeout {
        /// The time limit of the run.
        limit: Duration,
        /// False when the process was not gone a moment after its kill.
        stopped: bool,
    },
    /// The process ended with a status that is not zero.
    Exit(String),
    /// The output of the process could not be read. The text is the error of the system.
    Read(String),
    /// The process wrote more than the output limit.
    TooMuchOutput(usize),
    /// The answer is not JSON. YAML is not supported.
    NotJson {
        /// The line of the JSON error.
        line: usize,
        /// The column of the JSON error.
        column: usize,
    },
    /// `expirationTimestamp` is not an RFC 3339 time.
    BadExpiry,
    /// The token has a character that an HTTP header cannot carry.
    BadToken,
    /// The answer has no token. A plugin that gives a client certificate is not supported.
    NoToken,
}

impl Display for PluginFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCommand => f.write_str("the kubeconfig gives no command"),
            Self::NotFound { searched } => write!(f, "not found in the {searched} folders of PATH"),
            Self::Start(reason) => write!(f, "cannot start: {reason}"),
            Self::Timeout { limit, stopped: true } => write!(f, "no answer in {limit:?}"),
            Self::Timeout { limit, stopped: false } => write!(f, "no answer in {limit:?}, and its process still runs"),
            Self::Exit(status) => write!(f, "stopped with {status}"),
            Self::Read(reason) => write!(f, "cannot read its answer: {reason}"),
            Self::TooMuchOutput(limit) => write!(f, "wrote more than {limit} bytes"),
            Self::NotJson { line, column } => write!(f, "the answer is not JSON (line {line}, column {column})"),
            Self::BadExpiry => f.write_str("the expiry time of the answer is not an RFC 3339 time"),
            Self::BadToken => f.write_str("the token has a character that an HTTP header cannot carry"),
            Self::NoToken => f.write_str("the answer has no token"),
        }
    }
}

/// One plugin run failed.
#[derive(Clone, PartialEq, Eq)]
pub struct PluginError {
    command: String,
    failure: PluginFailure,
    detail: String,
}

impl PluginError {
    pub(crate) fn new(command: &str, failure: PluginFailure, detail: String) -> Self {
        Self { command: command.to_owned(), failure, detail }
    }

    /// Why the run gave no token.
    pub fn failure(&self) -> &PluginFailure {
        &self.failure
    }

    /// The text for the user: what the plugin wrote to stderr, or the install hint of the
    /// kubeconfig. Show it in the UI only, and never write it to a log.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "credential plugin `{}`: {}", self.command, self.failure)
    }
}

impl fmt::Debug for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PluginError({self}; {} characters of detail)", self.detail.chars().count())
    }
}

impl std::error::Error for PluginError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_text_has_the_failure_type_and_never_the_text_of_the_plugin() {
        let stderr = "Error: Unable to initialize DigitalOcean API client: access token is required.";
        let error = PluginError::new("doctl", PluginFailure::Exit("exit status: 7".to_owned()), stderr.to_owned());
        assert_eq!(error.to_string(), "credential plugin `doctl`: stopped with exit status: 7");
        assert_eq!(format!("{error:?}"), format!("PluginError({error}; {} characters of detail)", stderr.len()));
        assert_eq!(error.detail(), stderr);
    }

    #[test]
    fn a_process_that_is_not_gone_shows_in_the_text() {
        let limit = Duration::from_secs(20);
        assert_eq!(PluginFailure::Timeout { limit, stopped: true }.to_string(), "no answer in 20s");
        assert!(PluginFailure::Timeout { limit, stopped: false }.to_string().ends_with("its process still runs"));
    }
}
