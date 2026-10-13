//! The CronJob: its health value.

use k8s_openapi::api::batch::v1::CronJob;

use super::health::{Assess, ObjectHealth};

impl Assess for CronJob {
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
