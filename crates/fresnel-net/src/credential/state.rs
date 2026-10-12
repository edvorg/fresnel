//! The state of the credential of one cluster, and the token that requests use.

use std::fmt::{self, Display};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use http::HeaderValue;

use super::error::PluginError;
use crate::clock::since;

/// Why the plugin runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunReason {
    /// The first run for the cluster.
    Start,
    /// The token comes near its expiry time.
    Expiry,
    /// The server refused the token (HTTP 401).
    Unauthorized,
    /// A new attempt after a failure.
    Retry,
}

/// Why the cluster has no usable credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialFailure {
    /// The plugin run failed.
    Plugin(PluginError),
    /// The server refused a token that the plugin gave a moment before, also on a core path.
    Refused,
    /// The credential task ended. The start function runs it again.
    Stopped,
}

impl CredentialFailure {
    /// The text of the plugin for the user. Show it in the UI only.
    pub fn detail(&self) -> &str {
        match self {
            Self::Plugin(error) => error.detail(),
            Self::Refused | Self::Stopped => "",
        }
    }
}

impl Display for CredentialFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plugin(error) => Display::fmt(error, f),
            Self::Refused => f.write_str("the server refused a new token of the credential plugin (HTTP 401)"),
            Self::Stopped => f.write_str("the credential task stopped"),
        }
    }
}

/// The state of the credential that the UI shows (plan section 8.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialState {
    /// No token and no plugin run.
    None,
    /// The plugin runs now.
    Running {
        /// Why the plugin runs.
        reason: RunReason,
        /// The start of the run.
        since: SystemTime,
        /// The failure before this run, so that the UI keeps its text during the run.
        last_failure: Option<CredentialFailure>,
    },
    /// A token is ready.
    Valid {
        /// The expiry time of the token, if the plugin gave one.
        until: Option<SystemTime>,
    },
    /// The last run gave no usable token.
    Failed {
        /// Why.
        failure: CredentialFailure,
        /// The count of failures in sequence.
        failures: u32,
        /// The time of the next run. "Retry now", a wake and a network change start it earlier.
        next_attempt: SystemTime,
    },
}

/// The text has no plugin output, so it is safe for a log and for the error of a request.
impl Display for CredentialState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("no credential yet"),
            Self::Running { last_failure: Some(failure), .. } => {
                write!(f, "the credential plugin runs again after a failure ({failure})")
            }
            Self::Running { last_failure: None, .. } => f.write_str("the credential plugin runs"),
            Self::Valid { .. } => f.write_str("the credential is valid"),
            Self::Failed { failure, .. } => Display::fmt(failure, f),
        }
    }
}

/// What the server said about a token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Standing {
    /// Nothing yet.
    Fresh,
    /// The plugin gave this token after the server refused the token before it.
    AnswersRefusal,
    /// The server accepted this token on a core path.
    Proven,
}

/// A token with the facts that the HTTP 401 rule needs.
#[derive(Debug)]
pub(crate) struct Issued {
    pub(crate) header: HeaderValue,
    /// Counts the tokens of the cluster. A request tells which token the server refused.
    pub(crate) generation: u64,
    pub(crate) standing: Standing,
    /// The wall clock time of the standing: the plugin run or the proof.
    pub(crate) since: SystemTime,
}

impl Issued {
    /// True when the standing is `valid_for` old or less. An old standing tells nothing.
    pub(crate) fn has_recent(&self, standing: Standing, now: SystemTime, valid_for: Duration) -> bool {
        self.standing == standing && since(now, self.since) <= valid_for
    }
}

/// What a request does after the server refused its token.
#[derive(Debug)]
pub(crate) enum Verdict {
    /// A newer token exists. The request goes out again with it.
    Replaced(Arc<Issued>),
    /// The server accepts the token on a core path, so the HTTP 401 is the answer of one
    /// path. Only that request fails.
    PathOnly,
    /// No token: the request fails with this as its error.
    Unavailable(CredentialFailure),
}

/// The state together with the token. One value, so that a request never sees a state and a
/// token that do not fit together.
#[derive(Clone, Debug)]
pub(crate) struct View {
    pub(crate) state: CredentialState,
    pub(crate) token: Option<Arc<Issued>>,
}

impl View {
    /// The answer for a request that needs a token. `None` means: wait.
    pub(crate) fn token(&self) -> Option<Result<Arc<Issued>, CredentialFailure>> {
        match (&self.token, &self.state) {
            (Some(token), _) => Some(Ok(Arc::clone(token))),
            (None, CredentialState::Failed { failure, .. }) => Some(Err(failure.clone())),
            (None, CredentialState::None | CredentialState::Running { .. } | CredentialState::Valid { .. }) => None,
        }
    }

    /// The answer for a request whose token of generation `refused` got HTTP 401. `None`
    /// means: wait, the credential task did not decide yet.
    pub(crate) fn verdict(&self, refused: u64, now: SystemTime, proof_valid: Duration) -> Option<Verdict> {
        match self.token()? {
            Ok(token) if token.generation > refused => Some(Verdict::Replaced(token)),
            Ok(token) if token.has_recent(Standing::Proven, now, proof_valid) => Some(Verdict::PathOnly),
            Ok(_) => None,
            Err(failure) => Some(Verdict::Unavailable(failure)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential::error::PluginFailure;

    const VALID: Duration = Duration::from_secs(30);

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn view(generation: u64, standing: Standing, since: SystemTime) -> View {
        let issued = Issued { header: HeaderValue::from_static("Bearer x"), generation, standing, since };
        View { state: CredentialState::Valid { until: None }, token: Some(Arc::new(issued)) }
    }

    #[test]
    fn a_request_waits_only_while_a_token_can_still_come() {
        let mut none = View { state: CredentialState::None, token: None };
        assert!(none.token().is_none(), "the first run did not start yet");
        none.state = CredentialState::Running { reason: RunReason::Start, since: at(0), last_failure: None };
        assert!(none.token().is_none());
        none.state = CredentialState::Failed { failure: CredentialFailure::Refused, failures: 1, next_attempt: at(2) };
        assert_eq!(none.token().expect("an answer").expect_err("a failure"), CredentialFailure::Refused);
        assert!(view(1, Standing::Fresh, at(0)).token().expect("an answer").is_ok());
    }

    #[test]
    fn the_verdict_follows_the_generation_and_the_proof() {
        let verdict = |view: View, refused| view.verdict(refused, at(40), VALID);
        assert!(
            matches!(verdict(view(2, Standing::Fresh, at(40)), 1), Some(Verdict::Replaced(token)) if token.generation == 2)
        );
        assert!(verdict(view(2, Standing::Fresh, at(40)), 2).is_none(), "the task did not decide yet");
        assert!(verdict(view(2, Standing::AnswersRefusal, at(40)), 2).is_none());
        assert!(matches!(verdict(view(2, Standing::Proven, at(10)), 2), Some(Verdict::PathOnly)));
        assert!(verdict(view(2, Standing::Proven, at(9)), 2).is_none(), "the proof is too old");
    }

    #[test]
    fn the_text_of_a_state_has_no_plugin_output() {
        let error =
            PluginError::new("doctl", PluginFailure::Exit("exit status: 1".to_owned()), "secret text".to_owned());
        let failure = CredentialFailure::Plugin(error);
        let failed = CredentialState::Failed { failure: failure.clone(), failures: 1, next_attempt: at(2) };
        let running =
            CredentialState::Running { reason: RunReason::Retry, since: at(2), last_failure: Some(failure.clone()) };
        for text in [failed.to_string(), format!("{failed:?}"), running.to_string(), format!("{running:?}")] {
            assert!(text.contains("stopped with exit status: 1") && !text.contains("secret"), "{text}");
        }
        assert_eq!(failure.detail(), "secret text");
    }
}
