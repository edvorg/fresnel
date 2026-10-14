//! The APIService: its description and its health value.

use std::borrow::Cow;

use k8s_openapi::kube_aggregator::pkg::apis::apiregistration::v1::APIService;

use super::health::{Condition as _, ObjectHealth, condition};
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

impl Describe for APIService {
    const ID: Kind = Kind::ApiService;
    const TITLE: &'static str = "APIServices";
    const SIDEBAR: Sidebar = Sidebar::Other;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Service", 200.0, |service| {
            // A group that the API server serves itself has no Service.
            let reference = service.spec.as_ref().and_then(|spec| spec.service.as_ref());
            reference.map_or(Value::Text(Cow::Borrowed("Local")), |reference| {
                let (namespace, name) = (reference.namespace.as_deref(), reference.name.as_deref());
                Value::Text(Cow::Owned(format!("{}/{}", namespace.unwrap_or_default(), name.unwrap_or_default())))
            })
        }),
        Column::text("Available", 200.0, available),
    ];
    const ACTIONS: &'static [Action] = &[];

    /// Failed when the API server cannot reach the group: each request to it fails.
    fn health(&self) -> ObjectHealth {
        let conditions = self.status.as_ref().and_then(|status| status.conditions.as_ref());
        match condition(conditions, "Available").map(|available| available.status.as_str()) {
            Some("True") => ObjectHealth::Healthy,
            Some("False") => ObjectHealth::Failed,
            _ => ObjectHealth::Progressing,
        }
    }
}

/// The condition `Available` as kubectl shows it: `True`, or the status with its reason.
fn available(service: &APIService) -> Value<'static> {
    let conditions = service.status.as_ref().and_then(|status| status.conditions.as_ref());
    let Some(available) = condition(conditions, "Available") else {
        return Value::Text(Cow::Borrowed("Unknown"));
    };
    match (available.status.as_str(), available.reason()) {
        ("True", _) | (_, "") => Value::Text(Cow::Owned(available.status.clone())),
        (status, reason) => Value::Text(Cow::Owned(format!("{status} ({reason})"))),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value as Json, json};

    use super::*;
    use crate::fixture;
    use crate::kinds::ObjectHealth::{Failed, Healthy, Progressing};

    #[test]
    fn a_group_that_the_server_cannot_reach_failed() {
        let service = |change: fn(&mut Json)| fixture::of_kinds::<APIService>("v1alpha1.fresnel.test", change);
        let absent = service(|_| {});
        let text = Value::Text(Cow::Borrowed("False (ServiceNotFound)"));
        assert_eq!((absent.health(), available(&absent)), (Failed, text), "its Service does not exist");
        let reached =
            service(|api| api["status"] = json!({ "conditions": [{ "type": "Available", "status": "True" }] }));
        assert_eq!((reached.health(), available(&reached)), (Healthy, Value::Text(Cow::Borrowed("True"))));
        let new = service(|api| api["status"] = json!({}));
        assert_eq!((new.health(), available(&new)), (Progressing, Value::Text(Cow::Borrowed("Unknown"))));
    }
}
