//! Suspend and resume, trigger now and run again: the Jobs and the CronJobs of the app get
//! the state of their twins after the same change of kubectl.

use fresnel_core::kinds::Kind;
use fresnel_core::ops::{JobState, Operation, Outcome, StepState};
use serde_json::Value as Json;

use crate::bed::{APP, Cluster, KUBECTL, normal};

/// A suspend and a resume are a patch of `spec.suspend`, as `kubectl patch` sends it.
pub(crate) async fn suspend_and_resume(b: &Cluster) {
    for (kind, name) in [(Kind::CronJob, "nightly"), (Kind::Job, "once")] {
        for (operation, value) in [(Operation::Suspend, "true"), (Operation::Resume, "false")] {
            let report = b.run(kind, name, operation).await;
            assert_eq!(report.state, JobState::Done, "{report:#?}");
            let patch = format!("{{\"spec\":{{\"suspend\":{value}}}}}");
            let resource = crate::bed::resource(kind);
            b.kubectl(&["patch", &resource, name, "--type", "merge", "--patch", &patch, "--namespace", KUBECTL]).await;
            b.same_as_kubectl(kind, name).await;
        }
    }
}

/// The Job of a CronJob is the Job of `kubectl create job --from=cronjob/…`.
pub(crate) async fn trigger(b: &Cluster) {
    let report = b.run(Kind::CronJob, "nightly", Operation::Trigger("manual-1".into())).await;
    assert_eq!(report.state, JobState::Done, "{report:#?}");
    b.kubectl(&["create", "job", "manual-1", "--from", "cronjob/nightly", "--namespace", KUBECTL]).await;
    b.same_as_kubectl(Kind::Job, "manual-1").await;
    let job = b.object(APP, Kind::Job, "manual-1").await.expect("the new Job");
    let cron_job = b.object(APP, Kind::CronJob, "nightly").await.expect("the CronJob");
    assert_eq!(job.pointer("/metadata/ownerReferences/0/uid"), cron_job.pointer("/metadata/uid"), "its owner");
}

/// A new Job from the manifest of the old one: a new UID and the state of a create of the
/// same manifest, as its twin has it.
pub(crate) async fn run_again(b: &Cluster) {
    let before = b.target(Kind::Job, "once").await;
    let report = b.run(Kind::Job, "once", Operation::RunAgain).await;
    assert_eq!(report.state, JobState::Done, "{report:#?}");
    let steps: Vec<&str> = report.steps.iter().map(|step| step.text.as_str()).collect();
    let expected = [
        "read the Job",
        "check the new Job with a dry run",
        "delete the old Job and its pods",
        "wait until the old Job is gone",
        "create the new Job",
    ];
    assert_eq!(steps, expected);
    assert!(report.steps.iter().all(|step| matches!(step.state, StepState::Done(_))), "{report:#?}");
    assert!(matches!(report.outcome, Outcome::Written(Some(_))));

    let after = b.object(APP, Kind::Job, "once").await.expect("the new Job");
    assert_ne!(after.pointer("/metadata/uid").and_then(Json::as_str), Some(&*before.uid), "a new object");
    let twin = b.object(KUBECTL, Kind::Job, "once").await.expect("the twin");
    assert_eq!(normal(after), normal(twin), "the new Job and the old twin");
    b.until("the new Job has its pod", || async { !b.names(APP, Kind::Pod, "app=once").await.is_empty() }).await;
}
