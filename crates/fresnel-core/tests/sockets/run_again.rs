//! "Run again" of a Job at its worst moment (research 08 section 5.3): the old Job is gone,
//! and the new one is not there yet. Each end of the job there keeps the manifest for the
//! user, and no write goes two times. A store of the Jobs that follows nothing ends the job
//! before the delete. The test publishes the store of the Jobs itself, as the watch would.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use fresnel_core::CoreError;
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::net::{ErrorClass, Limits, NetSettings};
use fresnel_core::ops::{
    self, Cancel, JobState, Operation, Order, Outcome, Parts, Progress, Report, StepState, Target,
};
use fresnel_core::published::{Disposal, Notice, Publisher};
use fresnel_core::store::{ObjectKey, Snapshot, StoreState};
use fresnel_testkit::Fault;
use http::Method;
use k8s_openapi::api::batch::v1::Job;
use serde_json::Value as Json;
use tokio::sync::mpsc;

use crate::bed::{Bed, CONTEXT};

const JOBS: &str = "/apis/batch/v1/namespaces/shop/jobs";
const ONCE: &str = "/apis/batch/v1/namespaces/shop/jobs/once";
const OLD: &str = r#"{"apiVersion":"batch/v1","kind":"Job","metadata":{"name":"once","namespace":"shop","uid":"j1","resourceVersion":"5"},"spec":{"template":{"spec":{"restartPolicy":"Never","containers":[{"name":"c","image":"pause"}]}}}}"#;
const WAIT: &str = "wait until the old Job is gone";
/// The longest time of one job of the test.
const LIMIT: Duration = Duration::from_secs(10);

/// The store of the Jobs of the cluster. The publisher drops before the thread of its bin.
struct Jobs {
    publisher: Publisher<Snapshot<Job>>,
    _disposal: Disposal,
}

impl Jobs {
    /// A store in this state with the old Job.
    fn start(bed: &Bed, state: StoreState) -> Self {
        let disposal = Disposal::start(&bed.tasks.tasks);
        let old: Job = serde_json::from_str(OLD).expect("a Job");
        Self {
            publisher: Publisher::new(snapshot(state, vec![old]), disposal.bin(), Notice::new()),
            _disposal: disposal,
        }
    }
}

fn snapshot(state: StoreState, jobs: Vec<Job>) -> Snapshot<Job> {
    let now = SystemTime::now();
    let items = jobs.into_iter().map(Arc::new).collect();
    Snapshot { generation: 1, state, since: now, last_contact: Some(now), listed: true, dropped: 0, items }
}

/// Gives each report of the job to the test.
struct Forward(Option<mpsc::UnboundedSender<Report>>);

impl Progress for Forward {
    fn show(&mut self, report: &Report) {
        if self.0.as_ref().is_some_and(|sender| sender.send(report.clone()).is_err()) {
            // The test stopped to read: no more reports go to it.
            self.0 = None;
        }
    }
}

/// Runs "run again" of the old Job. `at_wait` runs one time, when the job waits for the
/// delete in the store. Gives the last report.
async fn run_again(bed: &Bed, jobs: &Jobs, cancel: Cancel, at_wait: impl FnOnce()) -> Report {
    let key = ObjectKey::new("shop", "once");
    let target = Target { cluster: ClusterId::new(CONTEXT), kind: Kind::Job, key, uid: "j1".into() };
    let order = Order::new(target, Operation::RunAgain).expect("an order");
    let parts = Parts {
        gateway: bed.link.gateway.clone(),
        deployments: None,
        stateful_sets: None,
        daemon_sets: None,
        jobs: Some(jobs.publisher.reader()),
        pods: None,
        cancel,
    };
    let (sender, mut reports) = mpsc::unbounded_channel();
    let run = async move { ops::run(&order, parts, &mut Forward(Some(sender))).await };
    let follow = async move {
        let (mut at_wait, mut last) = (Some(at_wait), Report::default());
        while let Some(report) = reports.recv().await {
            let waits = report.steps.last().is_some_and(|step| step.text == WAIT && step.state == StepState::Running);
            if let Some(event) = at_wait.take_if(|_| waits) {
                event();
            }
            last = report;
        }
        last
    };
    let (state, last) = tokio::time::timeout(LIMIT, async { tokio::join!(run, follow) }).await.expect("in time");
    assert_eq!(state, last.state, "the last report has the final state");
    last
}

/// The count of each request for the Jobs: the GETs and DELETEs of the old Job, the dry
/// runs and the creates.
fn sent(bed: &Bed) -> [usize; 4] {
    let requests = bed.server.requests();
    let count = |method: Method, path: &str, dry: Option<bool>| {
        let dry_run = |uri: &http::Uri| uri.query().is_some_and(|query| query.contains("dryRun=All"));
        let fits = |seen: &&fresnel_testkit::SeenRequest| {
            seen.method == method && seen.uri.path() == path && dry.is_none_or(|dry| dry_run(&seen.uri) == dry)
        };
        requests.iter().filter(fits).count()
    };
    [
        count(Method::GET, ONCE, None),
        count(Method::DELETE, ONCE, None),
        count(Method::POST, JOBS, Some(true)),
        count(Method::POST, JOBS, Some(false)),
    ]
}

/// The manifest of the outcome: the old Job with no field of the server.
fn assert_manifest(report: &Report) {
    let Outcome::Manifest(text) = &report.outcome else { panic!("the manifest: {report:#?}") };
    let manifest: Json = serde_json::from_str(text).expect("the JSON of the manifest");
    assert_eq!(manifest.pointer("/metadata/name"), Some(&Json::from("once")));
    assert_eq!(manifest.pointer("/metadata/uid"), None, "{text}");
}

fn wait_step(report: &Report) -> Option<&StepState> {
    report.steps.iter().find(|step| step.text == WAIT).map(|step| &step.state)
}

#[tokio::test]
async fn a_store_that_stops_or_a_cancel_after_the_delete_ends_the_job_with_the_manifest_and_no_create() {
    let bed = Bed::start(NetSettings::default()).await;
    bed.server.set_body(ONCE, OLD);

    // The cluster disconnects while the job waits: the store publishes its last state.
    let jobs = Jobs::start(&bed, StoreState::Live);
    let stop = || {
        jobs.publisher
            .close(|last| snapshot(StoreState::Stopped, last.items.iter().map(|(_, job)| Job::clone(job)).collect()))
    };
    let report = run_again(&bed, &jobs, Cancel::pair().1, stop).await;
    let text = "the store of the kind Job stopped: the cluster is not connected now";
    assert_eq!(report.state.error().map(ToString::to_string).as_deref(), Some(text), "{report:#?}");
    assert_eq!(wait_step(&report), Some(&StepState::Failed));
    assert_manifest(&report);
    assert_eq!(sent(&bed), [1, 1, 1, 0], "a read, a delete, a dry run and no create");

    // The user cancels the wait.
    let jobs = Jobs::start(&bed, StoreState::Live);
    let (cancel, signal) = Cancel::pair();
    let report = run_again(&bed, &jobs, signal, || {
        cancel.send_replace(true);
    })
    .await;
    assert_eq!(report.state, JobState::Cancelled, "{report:#?}");
    assert_eq!(wait_step(&report), Some(&StepState::Stopped));
    assert_manifest(&report);
    assert_eq!(sent(&bed), [2, 2, 2, 0], "no create after a cancel");
}

#[tokio::test]
async fn a_create_with_no_answer_after_the_delete_goes_out_one_time_and_keeps_the_manifest() {
    let limits = Limits { write: Duration::from_millis(500), get: Duration::from_millis(500), ..Limits::default() };
    let bed = Bed::start(NetSettings { limits, ..NetSettings::default() }).await;
    bed.server.set_body(ONCE, OLD);
    let jobs = Jobs::start(&bed, StoreState::Live);

    // The watch sees the delete, and from then the server answers no request for the Jobs.
    let gone = || {
        bed.server.add_fault(Fault::no_answer().on_path(JOBS));
        jobs.publisher.publish(snapshot(StoreState::Live, Vec::new()));
    };
    let report = run_again(&bed, &jobs, Cancel::pair().1, gone).await;
    let JobState::Unknown(error) = &report.state else { panic!("unknown: {report:#?}") };
    assert_eq!(error.class(), ErrorClass::Timeout, "{error}");
    let last: Vec<&str> = report.steps.iter().rev().take(2).map(|step| step.text.as_str()).collect();
    assert_eq!(last, ["read the object, because the result of the write is not known", "create the new Job"]);
    assert_manifest(&report);
    assert_eq!(sent(&bed), [2, 1, 1, 1], "the create went out one time, and a read followed it");
}

#[tokio::test]
async fn a_store_of_the_jobs_that_follows_nothing_ends_the_job_before_the_delete() {
    let bed = Bed::start(NetSettings::default()).await;
    bed.server.set_body(ONCE, OLD);
    let reason = CoreError::new(ErrorClass::Forbidden, "jobs is forbidden (HTTP 403)");
    let jobs = Jobs::start(&bed, StoreState::Forbidden { reason });

    let report = run_again(&bed, &jobs, Cancel::pair().1, || panic!("no wait")).await;
    let text = "the store of the kind Job follows no object: forbidden: jobs is forbidden (HTTP 403)";
    assert_eq!(report.state.error().map(ToString::to_string).as_deref(), Some(text), "{report:#?}");
    assert!(report.steps.is_empty(), "no step started: {report:#?}");
    assert_eq!(report.outcome, Outcome::None);
    assert_eq!(sent(&bed), [0, 0, 0, 0], "no request");
}
