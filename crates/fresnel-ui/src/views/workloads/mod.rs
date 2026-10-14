//! The detail sections of the workloads (plan section 11.4): Deployment, StatefulSet,
//! DaemonSet, ReplicaSet, ReplicationController, Job and CronJob. The content of each one is
//! the union of Headlamp and Lens (`research/01` section 3.4.2, `research/02` section 4.2).
//!
//! Each kind has its details (replicas, strategy, selector) and its parts: the pods with
//! their state as links, the pod template, and the revisions with a rollback for the kinds
//! that have them, or the last runs of a CronJob.
//!
//! - `deployment`, `stateful_set`, `daemon_set`, `replica_set` (with the ReplicationController),
//!   `job`, `cron_job`: one file for each kind.
//! - `pods`: the pods of a workload. `revisions`: the revisions with a rollback.

pub(crate) mod cron_job;
pub(crate) mod daemon_set;
pub(crate) mod deployment;
pub(crate) mod job;
mod pods;
pub(crate) mod replica_set;
mod revisions;
pub(crate) mod stateful_set;

use egui::Ui;
use k8s_openapi::api::core::v1::PodTemplateSpec;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;

use crate::views::detail::fields::{NONE, heading, text_field};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;
use crate::views::pod_spec::{self, scheduling};

/// The counts of the pods of a workload in one line: "3 desired, 2 ready, 3 up to date".
/// A count that the status does not have yet is 0, as kubectl prints it.
fn counts(ui: &mut Ui, cx: &Context<'_>, counts: &[(&str, Option<i32>)]) {
    let text: Vec<String> = counts.iter().map(|(label, count)| format!("{} {label}", count.unwrap_or(0))).collect();
    text_field(ui, cx.palette, "Replicas", &text.join(", "));
}

/// A line with a value that the spec can have, or the mark of no value.
fn line(ui: &mut Ui, cx: &Context<'_>, label: &str, value: Option<String>) {
    text_field(ui, cx.palette, label, value.as_deref().unwrap_or(NONE));
}

/// The text of a label selector, as kubectl prints it: `app=web,tier in (a,b)`.
fn selector(selector: Option<&LabelSelector>) -> String {
    let Some(selector) = selector else {
        return NONE.to_owned();
    };
    let labels = selector.match_labels.iter().flatten().map(|(key, value)| format!("{key}={value}"));
    let expressions = selector.match_expressions.iter().flatten().map(|expression| {
        let values = expression.values.as_deref().unwrap_or_default().join(",");
        match expression.operator.as_str() {
            "In" => format!("{} in ({values})", expression.key),
            "NotIn" => format!("{} notin ({values})", expression.key),
            "Exists" => expression.key.clone(),
            "DoesNotExist" => format!("!{}", expression.key),
            other => format!("{} {other} ({values})", expression.key),
        }
    });
    let text: Vec<String> = labels.chain(expressions).collect();
    if text.is_empty() { "each object".to_owned() } else { text.join(",") }
}

/// The pod template: where and as whom its pods run, its containers and its volumes.
fn template(ui: &mut Ui, cx: &Context<'_>, template: Option<&PodTemplateSpec>) -> Option<Link> {
    let spec = template.and_then(|template| template.spec.as_ref())?;
    heading(ui, cx.palette, "POD TEMPLATE");
    let place = cx.place();
    let clicked = scheduling::show(ui, &place, spec);
    pod_spec::show(ui, &place, spec, None).or(clicked)
}

/// A number of seconds, or the default of the server.
fn seconds(value: Option<i32>, default: Option<i32>) -> Option<String> {
    value.or(default).map(|seconds| format!("{seconds} s"))
}

/// The heading of the details of a kind.
fn details_heading(ui: &mut Ui, cx: &Context<'_>) {
    heading(ui, cx.palette, "DETAILS");
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_selector_reads_as_the_text_of_kubectl() {
        let selector: LabelSelector = serde_json::from_value(json!({"matchLabels": {"app": "web"},
            "matchExpressions": [{"key": "tier", "operator": "In", "values": ["a", "b"]},
                {"key": "canary", "operator": "DoesNotExist"}]}))
        .expect("a selector");
        assert_eq!(super::selector(Some(&selector)), "app=web,tier in (a,b),!canary");
        assert_eq!(super::selector(Some(&LabelSelector::default())), "each object");
        assert_eq!(super::selector(None), NONE);
    }
}
