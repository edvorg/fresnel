//! The CronJob: its description and its health value.

use k8s_openapi::api::batch::v1::CronJob;
use k8s_openapi::api::core::v1::PodSpec;

use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for CronJob {
    const ID: Kind = Kind::CronJob;
    const TITLE: &'static str = "CronJobs";
    const SIDEBAR: Sidebar = Sidebar::Workloads;
    const TIER: Tier = Tier::A;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Schedule", 110.0, |job| Value::text(job.spec.as_ref().map(|spec| spec.schedule.as_str()))),
        Column::text("Time zone", 110.0, |job| {
            Value::text(job.spec.as_ref().and_then(|spec| spec.time_zone.as_deref()))
        }),
        Column::text("Suspended", 70.0, |job| {
            Value::Flag(job.spec.as_ref().and_then(|spec| spec.suspend) == Some(true))
        }),
        Column::number("Active", 56.0, |job| {
            let active = job.status.as_ref().and_then(|status| status.active.as_ref());
            Value::Number(read::number_of(active.map_or(0, Vec::len)))
        }),
        Column::number("Last schedule", 100.0, |job| {
            Value::time(job.status.as_ref().and_then(|status| status.last_schedule_time.as_ref()))
        }),
        Column::text("Containers", 140.0, |job: &Self| read::containers(pod_spec(job))).hidden(),
        Column::text("Images", 220.0, |job| read::images(pod_spec(job))),
    ];
    const ACTIONS: &'static [Action] = &[Action::Trigger, Action::Suspend, Action::Resume];

    fn search<'a>(&'a self, texts: &mut Vec<&'a str>) {
        read::search_images(pod_spec(self), texts);
    }

    /// A CronJob that the user suspended, that never ran or whose Job runs now is
    /// healthy. A warning tells that the last Job of the schedule ended with no success:
    /// the last start is later than the last success.
    fn health(&self) -> ObjectHealth {
        let suspended = self.spec.as_ref().and_then(|spec| spec.suspend) == Some(true);
        let Some(status) = self.status.as_ref().filter(|_| !suspended) else {
            return ObjectHealth::Healthy;
        };
        let runs = status.active.as_ref().is_some_and(|active| !active.is_empty());
        let started = status.last_schedule_time.as_ref();
        if !runs && started.is_some() && status.last_successful_time.as_ref() < started {
            ObjectHealth::Warning
        } else {
            ObjectHealth::Healthy
        }
    }
}

fn pod_spec(job: &CronJob) -> Option<&PodSpec> {
    let template = job.spec.as_ref().and_then(|spec| spec.job_template.spec.as_ref()).map(|spec| &spec.template);
    read::pod_spec(template)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixture;
    use crate::kinds::ObjectHealth::{Healthy, Warning};

    #[test]
    fn a_cron_job_warns_when_its_last_job_ended_with_no_success() {
        assert_eq!(fixture::one::<CronJob>("tick").health(), Healthy, "its one Job is from the scene and completed");
        assert_eq!(fixture::one::<CronJob>("paused").health(), Healthy);

        let health = |name: &str, status: serde_json::Value| {
            fixture::changed::<CronJob>(name, |job| job["status"] = status).health()
        };
        let (early, late) = ("2026-01-01T00:00:00Z", "2026-01-01T00:00:09Z");
        assert_eq!(health("tick", json!({ "lastScheduleTime": early, "lastSuccessfulTime": late })), Healthy);
        assert_eq!(health("tick", json!({ "lastScheduleTime": late, "lastSuccessfulTime": early })), Warning);
        assert_eq!(health("tick", json!({ "lastScheduleTime": late })), Warning, "no Job of it completed");
        let runs = json!({ "lastScheduleTime": late, "active": [{ "name": "tick-1" }] });
        assert_eq!(health("tick", runs), Healthy, "the Job of the last start runs");
        assert_eq!(health("paused", json!({ "lastScheduleTime": late })), Healthy, "suspended by the user");
        assert_eq!(CronJob::default().health(), Healthy, "it never ran");
    }
}
