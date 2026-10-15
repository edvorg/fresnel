//! The sections of a HorizontalPodAutoscaler (`research/01` section 3.4.5, `research/02`
//! section 4.3): its target as a link, the range of its replicas, the current and the
//! desired count, its last scale and its behavior; then each metric with its current value
//! and its target, in the words of `kubectl describe`.
//!
//! The status has the current value of the metric at the same place as the spec, as
//! kubectl reads it. The conditions are in the section of each kind.

use egui::Ui;
use k8s_openapi::api::autoscaling::v2::{
    HPAScalingRules, HorizontalPodAutoscaler, MetricSpec, MetricStatus, MetricTarget, MetricValueStatus,
};

use crate::views::detail::fields::{field, heading};
use crate::views::detail::lines::{details_heading, line, none, selector, table};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;
use crate::views::pod_spec::texts::at;

/// The text of kubectl for a current value that the autoscaler does not know yet.
const UNKNOWN: &str = "<unknown>";

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, autoscaler: &HorizontalPodAutoscaler) -> Option<Link> {
    let (spec, status) = (autoscaler.spec.as_ref(), autoscaler.status.as_ref());
    details_heading(ui, cx);
    let mut clicked = None;
    if let Some(target) = spec.map(|spec| &spec.scale_target_ref) {
        field(ui, cx.palette, "Target", |ui| {
            clicked = cx.reference(ui, &target.kind, &cx.id.key.namespace, &target.name);
        });
    }
    let range = spec.map(|spec| format!("{} to {}", spec.min_replicas.unwrap_or(1), spec.max_replicas));
    line(ui, cx, "Replicas", range);
    let counts = status
        .map(|status| format!("{} current, {} desired", status.current_replicas.unwrap_or(0), status.desired_replicas));
    line(ui, cx, "Pods", counts);
    line(
        ui,
        cx,
        "Last scale",
        status.and_then(|status| status.last_scale_time.as_ref()).map(|time| at(time, cx.clock)),
    );
    let behavior = spec.and_then(|spec| spec.behavior.as_ref());
    if let Some(behavior) = behavior {
        line(ui, cx, "Scale up", behavior.scale_up.as_ref().map(rules));
        line(ui, cx, "Scale down", behavior.scale_down.as_ref().map(rules));
    }
    ui.add_space(10.0);
    heading(ui, cx.palette, "METRICS");
    let specs = spec.and_then(|spec| spec.metrics.as_deref()).unwrap_or_default();
    let statuses = status.and_then(|status| status.current_metrics.as_deref()).unwrap_or_default();
    let rows: Vec<[String; 3]> =
        specs.iter().enumerate().map(|(place, spec)| metric(spec, statuses.get(place))).collect();
    if rows.is_empty() {
        none(ui, cx);
    }
    table(ui, cx, "metrics", &["Metric", "Current", "Target"], &rows, |ui, row| {
        for text in row {
            ui.label(text.as_str());
        }
    });
    ui.add_space(10.0);
    clicked
}

/// "stabilization 300 s, the Max of: Pods 4 each 15 s, Percent 100 each 15 s".
fn rules(rules: &HPAScalingRules) -> String {
    let window = rules.stabilization_window_seconds.map(|seconds| format!("stabilization {seconds} s"));
    let policies: Vec<String> = rules
        .policies
        .iter()
        .flatten()
        .map(|policy| format!("{} {} each {} s", policy.type_, policy.value, policy.period_seconds))
        .collect();
    let select = rules.select_policy.as_deref().unwrap_or("Max");
    let policies = (!policies.is_empty()).then(|| format!("the {select} of: {}", policies.join(", ")));
    let parts: Vec<String> = window.into_iter().chain(policies).collect();
    if parts.is_empty() { "the defaults of the server".to_owned() } else { parts.join(", ") }
}

/// The name of a metric, its current value and its target, as `kubectl describe` prints
/// them: `resource cpu on pods (as a percentage of request)`, `41% (20m)`, `80%`.
fn metric(spec: &MetricSpec, status: Option<&MetricStatus>) -> [String; 3] {
    if let Some(resource) = &spec.resource {
        let current = status.and_then(|status| status.resource.as_ref()).map(|status| &status.current);
        let name = format!("resource {} on pods", resource.name);
        return resource_row(name, &resource.target, current);
    }
    if let Some(resource) = &spec.container_resource {
        let current = status.and_then(|status| status.container_resource.as_ref()).map(|status| &status.current);
        let name = format!("resource {} of container \"{}\" on pods", resource.name, resource.container);
        return resource_row(name, &resource.target, current);
    }
    if let Some(pods) = &spec.pods {
        let current = status.and_then(|status| status.pods.as_ref()).map(|status| &status.current);
        return [format!("\"{}\" on pods", pods.metric.name), value(current), target(&pods.target)];
    }
    if let Some(object) = &spec.object {
        let current = status.and_then(|status| status.object.as_ref()).map(|status| &status.current);
        let on = format!("{}/{}", object.described_object.kind, object.described_object.name);
        let name = format!("\"{}\" on {on} ({})", object.metric.name, form(&object.target));
        return [name, value(current), target(&object.target)];
    }
    if let Some(external) = &spec.external {
        let current = status.and_then(|status| status.external.as_ref()).map(|status| &status.current);
        let with = external.metric.selector.as_ref().map(|labels| format!(" with {}", selector(Some(labels))));
        let name = format!("\"{}\"{} ({})", external.metric.name, with.unwrap_or_default(), form(&external.target));
        return [name, value(current), target(&external.target)];
    }
    [format!("a metric of the type {}", spec.type_), UNKNOWN.to_owned(), UNKNOWN.to_owned()]
}

/// A resource metric: a percent of the requests of the pods, or an average value.
fn resource_row(name: String, goal: &MetricTarget, current: Option<&MetricValueStatus>) -> [String; 3] {
    let Some(percent) = goal.average_utilization else {
        return [name, value(current), target(goal)];
    };
    let now = current.and_then(|current| current.average_utilization).map(|now| format!("{now}%"));
    let average = current.and_then(|current| current.average_value.as_ref()).map(|value| format!(" ({})", value.0));
    let now = now.map_or_else(|| UNKNOWN.to_owned(), |now| format!("{now}{}", average.unwrap_or_default()));
    [format!("{name} (as a percentage of request)"), now, format!("{percent}%")]
}

/// "target value" or "target average value".
fn form(target: &MetricTarget) -> &'static str {
    if target.average_value.is_some() { "target average value" } else { "target value" }
}

fn target(target: &MetricTarget) -> String {
    let quantity = target.average_value.as_ref().or(target.value.as_ref());
    quantity.map_or_else(|| UNKNOWN.to_owned(), |quantity| quantity.0.clone())
}

fn value(current: Option<&MetricValueStatus>) -> String {
    let quantity = current.and_then(|current| current.average_value.as_ref().or(current.value.as_ref()));
    quantity.map_or_else(|| UNKNOWN.to_owned(), |quantity| quantity.0.clone())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn each_type_of_metric_reads_as_kubectl_describe_tells_it() {
        let specs: Vec<MetricSpec> = serde_json::from_value(json!([
            {"type": "Resource", "resource": {"name": "cpu", "target": {"type": "Utilization", "averageUtilization": 80}}},
            {"type": "Pods", "pods": {"metric": {"name": "queue"}, "target": {"type": "AverageValue", "averageValue": "5"}}},
            {"type": "Object", "object": {"describedObject": {"kind": "Service", "name": "web"},
                "metric": {"name": "requests"}, "target": {"type": "Value", "value": "10k"}}},
            {"type": "External", "external": {"metric": {"name": "lag", "selector": {"matchLabels": {"topic": "a"}}},
                "target": {"type": "AverageValue", "averageValue": "30"}}},
        ]))
        .expect("metrics");
        let statuses: Vec<MetricStatus> = serde_json::from_value(json!([
            {"type": "Resource", "resource": {"name": "cpu", "current": {"averageUtilization": 41, "averageValue": "20m"}}},
        ]))
        .expect("statuses");
        let rows: Vec<[String; 3]> =
            specs.iter().enumerate().map(|(place, spec)| metric(spec, statuses.get(place))).collect();
        let expected = [
            ["resource cpu on pods (as a percentage of request)", "41% (20m)", "80%"],
            ["\"queue\" on pods", UNKNOWN, "5"],
            ["\"requests\" on Service/web (target value)", UNKNOWN, "10k"],
            ["\"lag\" with topic=a (target average value)", UNKNOWN, "30"],
        ];
        assert_eq!(rows, expected.map(|row| row.map(str::to_owned)));
    }

    #[test]
    fn a_behavior_tells_its_window_and_its_policies() {
        let up: HPAScalingRules = serde_json::from_value(json!({"stabilizationWindowSeconds": 0, "selectPolicy": "Max",
            "policies": [{"type": "Pods", "value": 4, "periodSeconds": 15}, {"type": "Percent", "value": 100, "periodSeconds": 15}]}))
        .expect("rules");
        assert_eq!(rules(&up), "stabilization 0 s, the Max of: Pods 4 each 15 s, Percent 100 each 15 s");
        assert_eq!(rules(&HPAScalingRules::default()), "the defaults of the server");
    }
}
