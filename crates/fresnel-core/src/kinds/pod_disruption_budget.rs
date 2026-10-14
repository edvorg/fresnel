//! The PodDisruptionBudget: its description and its health value.

use std::borrow::Cow;

use k8s_openapi::api::policy::v1::{PodDisruptionBudget, PodDisruptionBudgetStatus};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

impl Describe for PodDisruptionBudget {
    const ID: Kind = Kind::PodDisruptionBudget;
    const TITLE: &'static str = "PodDisruptionBudgets";
    const SIDEBAR: Sidebar = Sidebar::Config;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::number("Min available", 90.0, |budget| {
            amount(budget.spec.as_ref().and_then(|spec| spec.min_available.as_ref()))
        }),
        Column::number("Max unavailable", 100.0, |budget| {
            amount(budget.spec.as_ref().and_then(|spec| spec.max_unavailable.as_ref()))
        }),
        Column::number("Allowed disruptions", 110.0, |budget| count(budget, |status| status.disruptions_allowed)),
        Column::number("Current healthy", 90.0, |budget| count(budget, |status| status.current_healthy)),
        Column::number("Desired healthy", 90.0, |budget| count(budget, |status| status.desired_healthy)),
    ];
    const ACTIONS: &'static [Action] = &[];

    /// A warning while fewer pods are healthy than the budget asks for: a drain then
    /// waits for this budget.
    fn health(&self) -> ObjectHealth {
        let status = self.status.as_ref();
        if status.is_some_and(|status| status.current_healthy < status.desired_healthy) {
            ObjectHealth::Warning
        } else {
            by_metadata(&self.metadata)
        }
    }
}

/// A count of pods or a percent of them.
fn amount(amount: Option<&IntOrString>) -> Value<'_> {
    match amount {
        Some(IntOrString::Int(count)) => Value::Number((*count).into()),
        Some(IntOrString::String(text)) => Value::Text(Cow::Borrowed(text)),
        None => Value::None,
    }
}

/// A count of the status. A budget that the controller did not see yet has no status.
fn count(budget: &PodDisruptionBudget, read: fn(&PodDisruptionBudgetStatus) -> i32) -> Value<'static> {
    budget.status.as_ref().map_or(Value::None, |status| Value::Number(read(status).into()))
}
