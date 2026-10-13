//! The Job: its health value.

use k8s_openapi::api::batch::v1::Job;

use super::health::{Assess, ObjectHealth, is_true};

impl Assess for Job {
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
