use std::time::Duration;

use serde_json::json;

use super::super::request::tests::target;
use super::*;
use crate::store::ObjectKey;

/// Each report that a job showed.
#[derive(Default)]
struct Seen(Vec<Report>);

impl Progress for Seen {
    fn show(&mut self, report: &Report) {
        self.0.push(report.clone());
    }
}

#[test]
fn each_change_of_a_job_shows_its_full_report_and_the_end_closes_the_running_step() {
    let mut seen = Seen::default();
    let mut job = Steps {
        target: target(Kind::Job, "shop", "once"),
        reads: target(Kind::Job, "shop", "once"),
        report: Report::default(),
        progress: &mut seen,
    };
    job.begin("read the Job");
    job.done("read");
    job.begin("delete the old Job");
    let state = job.close(JobState::Unknown(CoreError::invalid("no answer")));
    assert_eq!(state, JobState::Unknown(CoreError::invalid("no answer")));
    let last = seen.0.last().expect("a report");
    let steps: Vec<(&str, &StepState)> = last.steps.iter().map(|step| (step.text.as_str(), &step.state)).collect();
    assert_eq!(steps, [("read the Job", &StepState::Done("read".into())), ("delete the old Job", &StepState::Failed)]);
    assert_eq!(seen.0.len(), 4, "begin, done, begin and the end");
    assert_eq!(seen.0[0].state, JobState::Pending);

    let mut seen = Seen::default();
    let mut job = Steps {
        target: target(Kind::Job, "shop", "once"),
        reads: target(Kind::Job, "shop", "once"),
        report: Report::default(),
        progress: &mut seen,
    };
    job.begin("wait until the old Job is gone");
    assert_eq!(job.close(JobState::Cancelled), JobState::Cancelled);
    assert_eq!(seen.0.last().map(|report| report.steps[0].state.clone()), Some(StepState::Stopped));
}

#[test]
fn the_answer_of_a_write_gives_its_result_for_the_mark_and_the_target_its_identity() {
    let answer = json!({"kind": "Scale", "metadata": {"name": "web", "namespace": "shop", "uid": "u1",
        "resourceVersion": "77"}, "spec": {"replicas": 3}});
    let result = written(&answer).expect("a result");
    assert_eq!(result.key(), &ObjectKey::new("shop", "web"));
    assert_eq!(version(&answer), ", version 77");
    assert_eq!(written(&json!({"kind": "Status", "status": "Success"})), None);
    let node = identity(&target(Kind::Node, "", "pool-a1"));
    assert_eq!((node.name.as_deref(), node.namespace, node.uid.as_deref()), (Some("pool-a1"), None, Some("uid-1")));
}

#[tokio::test(start_paused = true)]
async fn a_cancel_ends_a_wait_and_so_does_the_drop_of_its_sender() {
    let (sender, mut cancel) = Cancel::pair();
    assert!(!cancel.is_set());
    let waited = tokio::time::timeout(Duration::from_secs(1), cancel.set()).await;
    assert!(waited.is_err(), "no cancel, no end");
    sender.send_replace(true);
    cancel.set().await;
    assert!(cancel.is_set());

    let (sender, mut cancel) = Cancel::pair();
    drop(sender);
    cancel.set().await;
}
