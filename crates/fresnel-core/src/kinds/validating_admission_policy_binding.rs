//! The ValidatingAdmissionPolicyBinding: its description.

use std::borrow::Cow;

use k8s_openapi::api::admissionregistration::v1::{ParamRef, ValidatingAdmissionPolicyBinding};

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for ValidatingAdmissionPolicyBinding {
    const ID: Kind = Kind::ValidatingAdmissionPolicyBinding;
    const TITLE: &'static str = "ValidatingAdmissionPolicyBindings";
    const SIDEBAR: Sidebar = Sidebar::Other;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Policy", 200.0, |binding| {
            Value::text(binding.spec.as_ref().and_then(|spec| spec.policy_name.as_deref()))
        }),
        Column::text("Param", 160.0, |binding| param(binding.spec.as_ref().and_then(|spec| spec.param_ref.as_ref()))),
        Column::text("Actions", 120.0, |binding| {
            let actions = binding.spec.as_ref().and_then(|spec| spec.validation_actions.as_ref());
            Value::list(actions.into_iter().flatten().map(String::as_str))
        }),
    ];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}

/// The parameter object of the binding, with the rule of kubectl: `namespace/name`, `*/name`
/// for each namespace, or the label selector.
fn param(reference: Option<&ParamRef>) -> Value<'static> {
    let Some(reference) = reference else {
        return Value::None;
    };
    match (reference.name.as_deref().filter(|name| !name.is_empty()), reference.namespace.as_deref()) {
        (Some(name), Some(namespace)) if !namespace.is_empty() => {
            Value::Text(Cow::Owned(format!("{namespace}/{name}")))
        }
        (Some(name), _) => Value::Text(Cow::Owned(format!("*/{name}"))),
        (None, _) => read::selector(reference.selector.as_ref()),
    }
}
