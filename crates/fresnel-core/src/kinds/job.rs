//! The Job: its description and its health value.

use std::borrow::Cow;
use std::time::SystemTime;

use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::PodSpec;

use super::health::{ObjectHealth, is_true};
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for Job {
    const ID: Kind = Kind::Job;
    const TITLE: &'static str = "Jobs";
    const SIDEBAR: Sidebar = Sidebar::Workloads;
    const TIER: Tier = Tier::A;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Status", 110.0, |job| Value::Text(Cow::Borrowed(status_text(job)))),
        Column::number("Completions", 90.0, completions),
        Column::number("Duration", 80.0, duration),
        Column::number("Succeeded", 70.0, |job: &Self| {
            read::count_or_zero(job.status.as_ref().and_then(|status| status.succeeded))
        })
        .hidden(),
        Column::number("Parallelism", 70.0, |job: &Self| {
            read::count(job.spec.as_ref().and_then(|spec| spec.parallelism))
        })
        .hidden(),
        Column::text("Suspended", 70.0, |job: &Self| {
            Value::Flag(job.spec.as_ref().and_then(|spec| spec.suspend) == Some(true))
        })
        .hidden(),
        Column::text("Conditions", 150.0, |job: &Self| {
            read::true_conditions(job.status.as_ref().and_then(|status| status.conditions.as_ref()))
        })
        .hidden(),
        Column::text("Containers", 140.0, |job: &Self| read::containers(pod_spec(job))).hidden(),
        Column::text("Images", 220.0, |job| read::images(pod_spec(job))),
    ];
    const ACTIONS: &'static [Action] =
        &[Action::RunAgain, Action::Suspend, Action::Resume, Action::Logs, Action::Forward];

    fn search<'a>(&'a self, texts: &mut Vec<&'a str>) {
        read::search_images(pod_spec(self), texts);
    }

    /// - Failed: the Job failed, or it met the rule for its failure and ends its pods.
    /// - Healthy: the Job completed, or the user suspended it.
    /// - Warning: the Job runs after a pod of it failed.
    /// - Progressing: the Job runs or waits for its start.
    fn health(&self) -> ObjectHealth {
        let status = self.status.as_ref();
        let conditions = status.and_then(|status| status.conditions.as_ref());
        if is_true(conditions, "Failed") || is_true(conditions, "FailureTarget") {
            ObjectHealth::Failed
        } else if is_true(conditions, "Complete") || is_true(conditions, "Suspended") {
            ObjectHealth::Healthy
        } else if status.and_then(|status| status.failed).unwrap_or(0) > 0 {
            ObjectHealth::Warning
        } else {
            ObjectHealth::Progressing
        }
    }
}

fn pod_spec(job: &Job) -> Option<&PodSpec> {
    read::pod_spec(job.spec.as_ref().map(|spec| &spec.template))
}

/// The state of the Job in a word, with the rule of kubectl: the first condition of this
/// sequence that is true, and else `Running`.
fn status_text(job: &Job) -> &'static str {
    let conditions = job.status.as_ref().and_then(|status| status.conditions.as_ref());
    if is_true(conditions, "Complete") {
        "Complete"
    } else if is_true(conditions, "Failed") {
        "Failed"
    } else if job.metadata.deletion_timestamp.is_some() {
        "Terminating"
    } else if is_true(conditions, "Suspended") {
        "Suspended"
    } else if is_true(conditions, "FailureTarget") {
        "FailureTarget"
    } else if is_true(conditions, "SuccessCriteriaMet") {
        "SuccessCriteriaMet"
    } else {
        "Running"
    }
}

/// The pods that succeeded, of the pods that the Job needs: `1/3`. With no count in the
/// spec, one success is enough, and kubectl adds the count of pods at the same time.
fn completions(job: &Job) -> Value<'static> {
    let spec = job.spec.as_ref();
    let succeeded = job.status.as_ref().and_then(|status| status.succeeded).unwrap_or(0);
    match (spec.and_then(|spec| spec.completions), spec.and_then(|spec| spec.parallelism).unwrap_or(0)) {
        (Some(completions), _) => Value::Ratio(succeeded.into(), completions.into()),
        (None, parallel) if parallel > 1 => Value::Text(Cow::Owned(format!("{succeeded}/1 of {parallel}"))),
        (None, _) => Value::Ratio(succeeded.into(), 1),
    }
}

/// The run time of a Job that ended, or the start of a Job that runs: a view shows the
/// time since it.
fn duration(job: &Job) -> Value<'static> {
    let status = job.status.as_ref();
    let time =
        |time: Option<&k8s_openapi::apimachinery::pkg::apis::meta::v1::Time>| time.map(|time| SystemTime::from(time.0));
    match (
        time(status.and_then(|status| status.start_time.as_ref())),
        time(status.and_then(|status| status.completion_time.as_ref())),
    ) {
        (None, _) => Value::None,
        (Some(start), None) => Value::Time(start),
        (Some(start), Some(end)) => end.duration_since(start).map_or(Value::None, Value::Span),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixture;
    use crate::kinds::ObjectHealth::{Failed, Healthy, Progressing, Warning};

    #[test]
    fn a_job_is_healthy_when_complete_or_suspended_and_failed_at_its_limit() {
        let health = |name: &str| fixture::one::<Job>(name).health();
        assert_eq!((health("once"), health("tick-manual")), (Healthy, Healthy));
        assert_eq!(health("loop-b"), Healthy, "the scene suspended it");
        assert_eq!(health("fails"), Failed);

        let once = |status: serde_json::Value| fixture::changed::<Job>("once", |job| job["status"] = status).health();
        assert_eq!(once(json!({ "active": 1 })), Progressing);
        assert_eq!(once(json!({})), Progressing, "it waits for its start");
        assert_eq!(once(json!({ "active": 1, "failed": 2 })), Warning, "it tries again");
        let target = json!({ "conditions": [{ "type": "FailureTarget", "status": "True" }], "active": 1 });
        assert_eq!(once(target), Failed, "the rule for the failure is met, and the pods end");
    }
}
