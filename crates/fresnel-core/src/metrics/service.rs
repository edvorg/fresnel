//! The APIService of `metrics.k8s.io`. A list that gets HTTP 503 tells only "service
//! unavailable"; the API server tells why in the `Available` condition of the APIService,
//! for example that it cannot reach the pod of metrics-server (research 11 section 4.6).

use std::fmt::{self, Display};
use std::sync::Arc;

use serde::Deserialize;

use crate::error::CoreError;

/// The name of the APIService that metrics-server registers.
const NAME: &str = "v1beta1.metrics.k8s.io";
/// The path of that APIService.
pub(crate) const PATH: &str = "/apis/apiregistration.k8s.io/v1/apiservices/v1beta1.metrics.k8s.io";

/// The fields of the APIService that the sampler reads. Each one can be absent.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct RawService {
    status: RawStatus,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawStatus {
    conditions: Vec<RawCondition>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawCondition {
    #[serde(rename = "type")]
    kind: String,
    status: String,
    reason: Option<String>,
    message: Option<String>,
}

/// What the APIService of the metrics tells about its backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    /// The `Available` condition.
    Condition {
        /// `True`, `False` or `Unknown`.
        status: Arc<str>,
        /// A word for the reason, for example `FailedDiscoveryCheck`.
        reason: Option<Arc<str>>,
        /// The text of the server.
        message: Option<Arc<str>>,
    },
    /// The APIService has no `Available` condition.
    NoCondition,
    /// The read of the APIService failed, for example because the rules of the user forbid it.
    Unread(CoreError),
}

impl Availability {
    /// The condition of a read of the APIService.
    pub(crate) fn of(read: Result<RawService, CoreError>) -> Self {
        let service = match read {
            Ok(service) => service,
            Err(error) => return Self::Unread(error),
        };
        let available = service.status.conditions.into_iter().find(|condition| condition.kind == "Available");
        let text = |text: Option<String>| text.filter(|text| !text.is_empty()).map(Arc::from);
        match available {
            Some(condition) => Self::Condition {
                status: condition.status.into(),
                reason: text(condition.reason),
                message: text(condition.message),
            },
            None => Self::NoCondition,
        }
    }
}

impl Display for Availability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Condition { status, reason, message } => {
                write!(f, "the APIService {NAME} has Available={status}")?;
                if let Some(reason) = reason {
                    write!(f, " ({reason})")?;
                }
                match message {
                    Some(message) => write!(f, ": {message}"),
                    None => Ok(()),
                }
            }
            Self::NoCondition => write!(f, "the APIService {NAME} has no Available condition"),
            Self::Unread(error) => write!(f, "the read of the APIService {NAME} failed: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn of(value: serde_json::Value) -> String {
        Availability::of(Ok(serde_json::from_value(value).expect("an APIService"))).to_string()
    }

    #[test]
    fn the_available_condition_gives_the_words_and_each_other_answer_its_own() {
        let failed = json!({ "status": { "conditions": [
            { "type": "Other", "status": "True" },
            { "type": "Available", "status": "False", "reason": "FailedDiscoveryCheck",
              "message": "failing or missing response from https://10.43.0.9:10250/apis/metrics.k8s.io/v1beta1" },
        ] } });
        assert_eq!(
            of(failed),
            "the APIService v1beta1.metrics.k8s.io has Available=False (FailedDiscoveryCheck): failing or missing \
             response from https://10.43.0.9:10250/apis/metrics.k8s.io/v1beta1"
        );
        let bare = json!({ "status": { "conditions": [{ "type": "Available", "status": "Unknown", "reason": "" }] } });
        assert_eq!(of(bare), "the APIService v1beta1.metrics.k8s.io has Available=Unknown");
        assert_eq!(of(json!({})), "the APIService v1beta1.metrics.k8s.io has no Available condition");
        let forbidden = CoreError::answered(403, "apiservices is forbidden (HTTP 403)");
        assert_eq!(
            Availability::of(Err(forbidden)).to_string(),
            "the read of the APIService v1beta1.metrics.k8s.io failed: apiservices is forbidden (HTTP 403)"
        );
    }
}
