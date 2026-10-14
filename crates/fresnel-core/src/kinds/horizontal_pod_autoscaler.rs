//! The HorizontalPodAutoscaler: its description and its health value.

use std::borrow::Cow;

use k8s_openapi::api::autoscaling::v2::{
    HorizontalPodAutoscaler, MetricSpec, MetricStatus, MetricTarget, MetricValueStatus,
};
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
fn resource_target(target: &MetricTarget, current: Option<&MetricValueStatus>) -> String {
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

#[cfg(test)]
mod tests {
    use serde_json::{Value as Json, json};

    use super::*;
    use crate::fixture::{self, condition};
    use crate::kinds::ObjectHealth::{Healthy, Warning};

    fn web(change: impl FnOnce(&mut Json)) -> HorizontalPodAutoscaler {
        fixture::of_kinds::<HorizontalPodAutoscaler>("web", change)
    }

    #[test]
    fn an_autoscaler_with_no_metrics_or_no_scale_is_a_warning_and_one_with_no_pod_by_intent_is_not() {
        assert_eq!(web(|_| {}).health(), Warning, "the test cluster has no metrics server");
        let disabled = |hpa: &mut Json| {
            condition(hpa, "ScalingActive")["reason"] = "ScalingDisabled".into();
            condition(hpa, "AbleToScale")["status"] = "True".into();
        };
        assert_eq!(web(disabled).health(), Healthy);
        let fine = |hpa: &mut Json| condition(hpa, "ScalingActive")["status"] = "True".into();
        assert_eq!(web(fine).health(), Healthy);
        let stuck = |hpa: &mut Json| {
            condition(hpa, "ScalingActive")["status"] = "True".into();
            condition(hpa, "AbleToScale")["status"] = "False".into();
        };
        assert_eq!(web(stuck).health(), Warning);
    }

    /// The fixture has a target of a percent with no current value. The other forms come
    /// from the rule of kubectl.
    #[test]
    fn each_form_of_a_target_has_the_text_of_kubectl() {
        let text_of = |metrics: Json, current: Json| {
            targets(&web(|hpa| {
                hpa["spec"]["metrics"] = metrics;
                hpa["status"]["currentMetrics"] = current;
            }))
        };
        let cpu = json!({ "name": "cpu", "target": { "type": "Utilization", "averageUtilization": 80 } });
        let now = json!({ "name": "cpu", "current": { "averageUtilization": 41 } });
        let percent = text_of(json!([{ "type": "Resource", "resource": cpu }]), json!([{ "resource": now }]));
        assert_eq!(percent, Value::texts(["cpu: 41%/80%".to_owned()]));
        let memory = json!({ "name": "memory", "target": { "type": "AverageValue", "averageValue": "100Mi" } });
        let queue = json!({ "metric": { "name": "queue" }, "target": { "type": "AverageValue", "averageValue": "5" } });
        let lag = json!({ "metric": { "name": "lag" }, "target": { "type": "Value", "value": "10" } });
        let metrics = json!([
            { "type": "Resource", "resource": memory },
            { "type": "Pods", "pods": queue },
            { "type": "External", "external": lag },
        ]);
        let texts = ["memory: <unknown>/100Mi", "<unknown>/5", "<unknown>/10"];
        assert_eq!(text_of(metrics, json!([])), Value::texts(texts.map(str::to_owned)));
    }
}
