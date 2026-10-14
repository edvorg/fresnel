//! The IngressClass: its description.

use std::borrow::Cow;

use k8s_openapi::api::networking::v1::IngressClass;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

/// The annotation that makes a class the default of the cluster.
const DEFAULT: &str = "ingressclass.kubernetes.io/is-default-class";

impl Describe for IngressClass {
    const ID: Kind = Kind::IngressClass;
    const TITLE: &'static str = "IngressClasses";
    const SIDEBAR: Sidebar = Sidebar::Network;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Controller", 200.0, |class| {
            Value::text(class.spec.as_ref().and_then(|spec| spec.controller.as_deref()))
        }),
        Column::text("Parameters", 200.0, |class| {
            // The form of kubectl: `Kind.group/name`.
            let parameters = class.spec.as_ref().and_then(|spec| spec.parameters.as_ref());
            parameters.map_or(Value::None, |parameters| {
                let group = parameters.api_group.as_deref().map(|group| format!(".{group}")).unwrap_or_default();
                Value::Text(Cow::Owned(format!("{}{group}/{}", parameters.kind, parameters.name)))
            })
        }),
        Column::text("Default", 70.0, |class| {
            let annotations = class.metadata.annotations.as_ref();
            Value::Flag(
                annotations.and_then(|annotations| annotations.get(DEFAULT)).is_some_and(|value| value == "true"),
            )
        }),
    ];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
