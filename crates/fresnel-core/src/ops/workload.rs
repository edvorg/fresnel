//! Scale, restart, suspend and resume: one patch each (research 08 sections 4.1, 4.2, 5.2).

use http::Request;
use k8s_openapi::jiff::Timestamp;
use serde_json::{Value as Json, json};

use super::Target;
use super::request::{self, PatchType};
use crate::error::CoreError;

/// The annotation of the pod template that `kubectl rollout restart` writes. The kube
/// helper `Api::restart` writes an other key, which kubectl and the other tools do not show.
pub(super) const RESTARTED_AT: &str = "kubectl.kubernetes.io/restartedAt";

/// A merge patch of the subresource `scale` with an absolute count, as `kubectl scale` with
/// no precondition sends it: the user gives the number, so the last write is the correct one.
pub(super) fn scale(target: &Target, replicas: i32) -> Result<Request<Vec<u8>>, CoreError> {
    if replicas < 0 {
        return Err(CoreError::invalid(format!("a count of pods cannot be {replicas}")));
    }
    request::patch(target, Some("scale"), PatchType::Merge, &json!({"spec": {"replicas": replicas}}))
}

/// The strategic patch of `kubectl rollout restart`: a new time in the annotation of the
/// pod template makes the controller replace each pod. kubectl writes the time to the
/// second, so a second restart in the same second changes nothing, as with kubectl.
pub(super) fn restart(target: &Target, now: Timestamp) -> Result<Request<Vec<u8>>, CoreError> {
    let at =
        Timestamp::from_second(now.as_second()).map_err(|_| CoreError::invalid("the clock gives no valid time"))?;
    let body = json!({"spec": {"template": {"metadata": {"annotations": {RESTARTED_AT: at.to_string()}}}}});
    request::patch(target, None, PatchType::Strategic, &body)
}

/// kubectl refuses the restart of a paused Deployment, because the controller does not
/// start a rollout for it. `object` is the workload as the server gave it.
pub(super) fn check_restart(object: &Json) -> Result<(), CoreError> {
    if object.pointer("/spec/paused") == Some(&Json::Bool(true)) {
        return Err(CoreError::invalid("can't restart paused deployment (run rollout resume first)"));
    }
    Ok(())
}

/// The merge patch of a suspend (`true`) or a resume (`false`) of a Job or a CronJob.
pub(super) fn suspend(target: &Target, suspend: bool) -> Result<Request<Vec<u8>>, CoreError> {
    request::patch(target, None, PatchType::Merge, &json!({"spec": {"suspend": suspend}}))
}

#[cfg(test)]
mod tests {
    use super::super::request::tests::{parts, target};
    use super::*;
    use crate::kinds::Kind;

    #[test]
    fn scale_is_a_merge_patch_of_the_subresource_with_the_count() {
        let (method, url, content, body) =
            parts(&scale(&target(Kind::StatefulSet, "shop", "db"), 3).expect("a request"));
        assert_eq!(method, "PATCH");
        assert_eq!(url, "/apis/apps/v1/namespaces/shop/statefulsets/db/scale?&fieldManager=fresnel");
        assert_eq!((content.as_str(), body), ("application/merge-patch+json", json!({"spec": {"replicas": 3}})));
        let rc = parts(&scale(&target(Kind::ReplicationController, "shop", "old"), 0).expect("a request"));
        assert_eq!(rc.1, "/api/v1/namespaces/shop/replicationcontrollers/old/scale?&fieldManager=fresnel");
        let error = scale(&target(Kind::Deployment, "shop", "web"), -1).expect_err("no negative count");
        assert_eq!(error.to_string(), "a count of pods cannot be -1");
    }

    #[test]
    fn restart_writes_the_annotation_of_kubectl_with_the_time_to_the_second() {
        let now: Timestamp = "2026-10-12T20:00:00.987Z".parse().expect("a time");
        let (method, url, content, body) =
            parts(&restart(&target(Kind::DaemonSet, "kube-system", "agent"), now).expect("a request"));
        assert_eq!(method, "PATCH");
        assert_eq!(url, "/apis/apps/v1/namespaces/kube-system/daemonsets/agent?&fieldManager=fresnel");
        assert_eq!(content, "application/strategic-merge-patch+json");
        let annotations = json!({"kubectl.kubernetes.io/restartedAt": "2026-10-12T20:00:00Z"});
        assert_eq!(body, json!({"spec": {"template": {"metadata": {"annotations": annotations}}}}));
    }

    #[test]
    fn a_paused_deployment_does_not_restart() {
        assert!(check_restart(&json!({"spec": {"paused": false}})).is_ok());
        assert!(check_restart(&json!({"spec": {}})).is_ok());
        let error = check_restart(&json!({"spec": {"paused": true}})).expect_err("paused");
        assert_eq!(error.to_string(), "can't restart paused deployment (run rollout resume first)");
    }

    #[test]
    fn suspend_and_resume_are_merge_patches_of_the_spec() {
        let (_, url, content, body) =
            parts(&suspend(&target(Kind::CronJob, "shop", "nightly"), true).expect("a request"));
        assert_eq!(url, "/apis/batch/v1/namespaces/shop/cronjobs/nightly?&fieldManager=fresnel");
        assert_eq!((content.as_str(), body), ("application/merge-patch+json", json!({"spec": {"suspend": true}})));
        let resume = parts(&suspend(&target(Kind::Job, "shop", "once"), false).expect("a request"));
        assert_eq!(
            (resume.1.as_str(), resume.3),
            ("/apis/batch/v1/namespaces/shop/jobs/once?&fieldManager=fresnel", json!({"spec": {"suspend": false}}))
        );
    }
}
