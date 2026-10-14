//! The HorizontalPodAutoscaler: its description and its health value.

use std::borrow::Cow;

use k8s_openapi::api::autoscaling::v2::{HorizontalPodAutoscaler, MetricSpec, MetricStatus, MetricTarget};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

use super::describe::by_metadata;
use super::health::{Condition as _, ObjectHealth, condition};
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

/// The text of kubectl for a current value that the autoscaler does not know yet.
const UNKNOWN: &str = "<unknown>";

impl Describe for HorizontalPodAutoscaler {
    const ID: Kind = Kind::HorizontalPodAutoscaler;
    const TITLE: &'static str = "HorizontalPodAutoscalers";
    const SIDEBAR: Sidebar = Sidebar::Config;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Reference", 160.0, |autoscaler| {
            let target = autoscaler.spec.as_ref().map(|spec| &spec.scale_target_ref);
            target.map_or(Value::None, |target| Value::Text(Cow::Owned(format!("{}/{}", target.kind, target.name))))
        }),
        Column::text("Targets", 180.0, targets),
        Column::number("Min pods", 64.0, |autoscaler| {
            read::count(autoscaler.spec.as_ref().and_then(|spec| spec.min_replicas))
        }),
        Column::number("Max pods", 64.0, |autoscaler| {
            read::count(autoscaler.spec.as_ref().map(|spec| spec.max_replicas))
        }),
        Column::number("Replicas", 64.0, |autoscaler| {
            read::count_or_zero(autoscaler.status.as_ref().and_then(|status| status.current_replicas))
        }),
        Column::text("Conditions", 160.0, |autoscaler| {
            read::true_conditions(autoscaler.status.as_ref().and_then(|status| status.conditions.as_ref()))
        }),
    ];
    const ACTIONS: &'static [Action] = &[];

    /// A warning when the autoscaler cannot scale its target or cannot read its metrics.
    /// A target with no pod by intent turns the scaling off, and that is no fault.
    fn health(&self) -> ObjectHealth {
        let conditions = self.status.as_ref().and_then(|status| status.conditions.as_ref());
        let is_false = |name: &str| condition(conditions, name).filter(|condition| condition.status == "False");
        let inactive = is_false("ScalingActive").is_some_and(|condition| condition.reason() != "ScalingDisabled");
        if is_false("AbleToScale").is_some() || inactive { ObjectHealth::Warning } else { by_metadata(&self.metadata) }
    }
}

/// The current value and the target of each metric, with the rule of kubectl, for
/// example `cpu: 41%/80%`.
fn targets(autoscaler: &HorizontalPodAutoscaler) -> Value<'static> {
    let specs = autoscaler.spec.as_ref().and_then(|spec| spec.metrics.as_ref());
    let statuses = autoscaler.status.as_ref().and_then(|status| status.current_metrics.as_ref());
    let current = |place: usize| statuses.and_then(|statuses| statuses.get(place));
    Value::texts(specs.into_iter().flatten().enumerate().map(|(place, spec)| target(spec, current(place))))
}

fn target(spec: &MetricSpec, status: Option<&MetricStatus>) -> String {
    let text = |quantity: Option<&Quantity>| quantity.map_or(UNKNOWN, |quantity| quantity.0.as_str()).to_owned();
    if let Some(resource) = &spec.resource {
        let current = status.and_then(|status| status.resource.as_ref()).map(|status| &status.current);
        return format!("{}: {}", resource.name, resource_target(&resource.target, current));
    }
    if let Some(resource) = &spec.container_resource {
        let current = status.and_then(|status| status.container_resource.as_ref()).map(|status| &status.current);
        return format!("{}: {}", resource.name, resource_target(&resource.target, current));
    }
    if let Some(pods) = &spec.pods {
        let current =
            status.and_then(|status| status.pods.as_ref()).and_then(|status| status.current.average_value.as_ref());
        return format!("{}/{}", text(current), text(pods.target.average_value.as_ref()));
    }
    let (target, current) = match (&spec.object, &spec.external) {
        (Some(object), _) => {
            (&object.target, status.and_then(|status| status.object.as_ref()).map(|status| &status.current))
        }
        (None, Some(external)) => {
            (&external.target, status.and_then(|status| status.external.as_ref()).map(|status| &status.current))
        }
        (None, None) => return "<unknown type>".to_owned(),
    };
    match &target.average_value {
        Some(average) => {
            format!("{}/{} (avg)", text(current.and_then(|current| current.average_value.as_ref())), average.0)
        }
        None => format!("{}/{}", text(current.and_then(|current| current.value.as_ref())), text(target.value.as_ref())),
    }
}

/// The target of a resource: an average value, or a percent of the requests of the pods.
fn resource_target(
    target: &MetricTarget,
    current: Option<&k8s_openapi::api::autoscaling::v2::MetricValueStatus>,
) -> String {
    match &target.average_value {
        Some(average) => {
            let now = current.and_then(|current| current.average_value.as_ref());
            format!("{}/{}", now.map_or(UNKNOWN, |now| now.0.as_str()), average.0)
        }
        None => {
            let now = current.and_then(|current| current.average_utilization);
            let now = now.map_or_else(|| UNKNOWN.to_owned(), |now| format!("{now}%"));
            let goal = target.average_utilization.map_or_else(|| "<auto>".to_owned(), |goal| format!("{goal}%"));
            format!("{now}/{goal}")
        }
    }
}
