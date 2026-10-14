//! The CertificateSigningRequest: its description and its health value.

use std::borrow::Cow;
use std::time::Duration;

use k8s_openapi::api::certificates::v1::CertificateSigningRequest;

use super::health::{ObjectHealth, condition};
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

impl Describe for CertificateSigningRequest {
    const ID: Kind = Kind::CertificateSigningRequest;
    const TITLE: &'static str = "CertificateSigningRequests";
    const SIDEBAR: Sidebar = Sidebar::Other;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Signer", 220.0, |request| Value::text(Some(&request.spec.signer_name))),
        Column::text("Requestor", 160.0, |request| Value::text(request.spec.username.as_deref())),
        Column::number("Requested duration", 100.0, |request| {
            let seconds = request.spec.expiration_seconds.and_then(|seconds| u64::try_from(seconds).ok());
            seconds.map_or(Value::None, |seconds| Value::Span(Duration::from_secs(seconds)))
        }),
        Column::text("Condition", 130.0, |request| Value::Text(Cow::Owned(state(request)))),
    ];
    const ACTIONS: &'static [Action] = &[];

    /// Failed when the request was denied or failed. Healthy after the signer issued the
    /// certificate, and on its way before.
    fn health(&self) -> ObjectHealth {
        let conditions = self.status.as_ref().and_then(|status| status.conditions.as_ref());
        if condition(conditions, "Denied").is_some() || condition(conditions, "Failed").is_some() {
            ObjectHealth::Failed
        } else if issued(self) {
            ObjectHealth::Healthy
        } else {
            ObjectHealth::Progressing
        }
    }
}

fn issued(request: &CertificateSigningRequest) -> bool {
    request
        .status
        .as_ref()
        .and_then(|status| status.certificate.as_ref())
        .is_some_and(|certificate| !certificate.0.is_empty())
}

/// The state of the request with the rule of kubectl: `Denied`, `Approved` or `Pending`,
/// then `Failed` and `Issued` when they apply. kubectl reads the type of a condition only.
fn state(request: &CertificateSigningRequest) -> String {
    let conditions = request.status.as_ref().and_then(|status| status.conditions.as_ref());
    let has = |name: &str| condition(conditions, name).is_some();
    let mut state = if has("Denied") {
        "Denied"
    } else if has("Approved") {
        "Approved"
    } else {
        "Pending"
    }
    .to_owned();
    if has("Failed") {
        state.push_str(",Failed");
    }
    if issued(request) {
        state.push_str(",Issued");
    }
    state
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::fixture;
    use crate::kinds::ObjectHealth::{Failed, Healthy, Progressing};

    fn reader(change: impl FnOnce(&mut Value)) -> CertificateSigningRequest {
        fixture::of_kinds::<CertificateSigningRequest>("fresnel-reader", change)
    }

    #[test]
    fn a_request_is_healthy_with_its_certificate_and_failed_when_denied() {
        let request = reader(|_| {});
        assert_eq!((request.health(), state(&request).as_str()), (Healthy, "Approved,Issued"));
        let waits = reader(|request| request["status"] = json!({}));
        assert_eq!((waits.health(), state(&waits).as_str()), (Progressing, "Pending"));
        let denied =
            reader(|request| request["status"] = json!({ "conditions": [{ "type": "Denied", "status": "True" }] }));
        assert_eq!((denied.health(), state(&denied).as_str()), (Failed, "Denied"));
        let failed = |request: &mut Value| request["status"]["conditions"][0]["type"] = "Failed".into();
        assert_eq!(state(&reader(failed)), "Pending,Failed,Issued");
    }
}
