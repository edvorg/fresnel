//! The ValidatingAdmissionPolicy: its description.

use std::borrow::Cow;

use k8s_openapi::api::admissionregistration::v1::ValidatingAdmissionPolicy;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for ValidatingAdmissionPolicy {
    const ID: Kind = Kind::ValidatingAdmissionPolicy;
    const TITLE: &'static str = "ValidatingAdmissionPolicies";
    const SIDEBAR: Sidebar = Sidebar::Other;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::number("Validations", 80.0, |policy| {
            let validations = policy.spec.as_ref().and_then(|spec| spec.validations.as_ref());
            Value::Number(read::number_of(validations.map_or(0, Vec::len)))
        }),
        Column::text("Param kind", 160.0, |policy| {
            // The form of kubectl: `apiVersion/kind`.
            let kind = policy.spec.as_ref().and_then(|spec| spec.param_kind.as_ref());
            kind.map_or(Value::None, |kind| {
                let (version, kind) = (kind.api_version.as_deref(), kind.kind.as_deref());
                Value::Text(Cow::Owned(format!("{}/{}", version.unwrap_or_default(), kind.unwrap_or_default())))
            })
        }),
    ];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
