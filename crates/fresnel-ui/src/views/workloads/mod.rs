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
//! - `pods`: the pods of a workload, and the table of pods of a Node. `revisions`: the
//!   revisions with a rollback.

pub(crate) mod cron_job;
pub(crate) mod daemon_set;
pub(crate) mod deployment;
pub(crate) mod job;
pub(crate) mod pods;
pub(crate) mod replica_set;
mod revisions;
pub(crate) mod stateful_set;

use egui::Ui;
use k8s_openapi::api::core::v1::PodTemplateSpec;

use crate::views::detail::fields::{heading, text_field};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;
use crate::views::pod_spec::{self, scheduling};

/// The counts of the pods of a workload in one line: "3 desired, 2 ready, 3 up to date".
/// A count that the status does not have yet is 0, as kubectl prints it.
fn counts(ui: &mut Ui, cx: &Context<'_>, counts: &[(&str, Option<i32>)]) {
    let text: Vec<String> = counts.iter().map(|(label, count)| format!("{} {label}", count.unwrap_or(0))).collect();
    text_field(ui, cx.palette, "Replicas", &text.join(", "));
}

/// The pod template: where and as whom its pods run, its containers and its volumes.
fn template(ui: &mut Ui, cx: &Context<'_>, template: Option<&PodTemplateSpec>) -> Option<Link> {
    let spec = template.and_then(|template| template.spec.as_ref())?;
    heading(ui, cx.palette, "POD TEMPLATE");
    let place = cx.place();
    let clicked = scheduling::show(ui, &place, spec);
    pod_spec::show(ui, &place, spec, None).or(clicked)
}
