//! The steps of each operation through the gateway of the cluster of its object.
//!
//! A procedure sends each write one time (design rule 6). A write with a result that is
//! not known ends the job, and a last step reads the object, so the user sees what is in
//! effect. A cancel ends a wait at once; a write in progress runs to its answer first.
//!
//! - `long`: the steps that wait for a store: the rollout status and "run again".
//! - `revisions`: the history and the rollback, which read the workload and its revisions.
//! - `drain`: the drain of a node, with a task for each of its pods.

mod drain;
mod long;
mod revisions;

use std::sync::Arc;

use http::Request;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::jiff::Timestamp;
use serde_json::Value as Json;
use tokio::sync::watch;

use super::state::{JobState, Outcome, Report, Step, StepState};
use super::{Operation, Order, Target, delete, job, node, request, workload};
use crate::cluster::Gateway;
use crate::error::CoreError;
use crate::kinds::Kind;
use crate::published::Reader;
use crate::store::{ObjectKey, Snapshot, Written};

/// Takes each new report of a job: the owner shows it.
pub trait Progress: Send {
    /// The job changed. The report is complete, not a difference.
    fn show(&mut self, report: &Report);
}

/// The signal of a cancel of one job. The owner keeps the sender.
#[derive(Clone, Debug)]
pub struct Cancel(watch::Receiver<bool>);

impl Cancel {
    /// A signal and the sender that sets it.
    pub fn pair() -> (watch::Sender<bool>, Self) {
        let (sender, receiver) = watch::channel(false);
        (sender, Self(receiver))
    }

    fn is_set(&self) -> bool {
        *self.0.borrow()
    }

    /// Ends when the owner sets the signal, or drops its sender: then no one waits for the job.
    async fn until_set(&mut self) {
        while !*self.0.borrow_and_update() {
            // An error tells that the sender is gone: no one waits for the job, a cancel too.
            if self.0.changed().await.is_err() {
                return;
            }
        }
    }
}

/// What a procedure needs besides its order.
#[derive(Clone)]
pub struct Parts {
    /// The gateway of the cluster of the object.
    pub gateway: Gateway,
    /// The stores whose objects a step follows. A store that the cluster does not have is
    /// `None`; a step that needs it fails with that reason.
    pub deployments: Option<Reader<Snapshot<Deployment>>>,
    /// The StatefulSets.
    pub stateful_sets: Option<Reader<Snapshot<StatefulSet>>>,
    /// The DaemonSets.
    pub daemon_sets: Option<Reader<Snapshot<DaemonSet>>>,
    /// The Jobs.
    pub jobs: Option<Reader<Snapshot<Job>>>,
    /// The Pods, whose end a drain waits for.
    pub pods: Option<Reader<Snapshot<Pod>>>,
    /// The cancel of the job.
    pub cancel: Cancel,
}

/// Runs an order to its final state. `progress` gets each change of the report.
pub async fn run(order: &Order, parts: Parts, progress: &mut impl Progress) -> JobState {
    let (target, reads) = (order.target.clone(), order.target.clone());
    let mut job = Steps { target, reads, report: Report::default(), progress };
    let state = match steps(order, &parts, &mut job).await {
        Ok(()) => JobState::Done,
        Err(state) => state,
    };
    job.close(state)
}

/// The report of a running job, which each change gives to the owner.
struct Steps<'a, P: Progress> {
    target: Target,
    /// The object that a write makes or changes: the target, or the Job of a trigger. A
    /// write with an unknown result reads it.
    reads: Target,
    report: Report,
    progress: &'a mut P,
}

impl<P: Progress> Steps<'_, P> {
    fn begin(&mut self, text: impl Into<String>) {
        self.report.steps.push(Step { text: text.into(), state: StepState::Running });
        self.progress.show(&self.report);
    }

    /// Ends the running step with the text of its result.
    fn done(&mut self, result: impl Into<String>) {
        self.end_step(StepState::Done(result.into()));
        self.progress.show(&self.report);
    }

    fn outcome(&mut self, outcome: Outcome) {
        self.report.outcome = outcome;
        self.progress.show(&self.report);
    }

    fn end_step(&mut self, state: StepState) {
        if let Some(step) = self.report.steps.last_mut().filter(|step| step.state == StepState::Running) {
            step.state = state;
        }
    }

    fn close(mut self, state: JobState) -> JobState {
        let step = match &state {
            JobState::Cancelled => StepState::Stopped,
            _ => StepState::Failed,
        };
        self.end_step(step);
        self.report.state = state.clone();
        self.progress.show(&self.report);
        state
    }
}

/// The steps of each operation. An error is the final state of the job.
async fn steps<P: Progress>(order: &Order, parts: &Parts, job: &mut Steps<'_, P>) -> Result<(), JobState> {
    let target = &order.target;
    if parts.gateway.cluster() != &target.cluster {
        let text = format!("a request for {} cannot go to {}", target.cluster, parts.gateway.cluster());
        return Err(JobState::NotSent(CoreError::invalid(text)));
    }
    if !order.operation.fits(target.kind) {
        return Err(JobState::NotSent(CoreError::invalid(format!(
            "a {} has no such operation",
            target.kind.info().name
        ))));
    }
    match &order.operation {
        Operation::Scale(replicas) => {
            write(parts, job, format!("set the count of pods to {replicas}"), workload::scale(target, *replicas)).await
        }
        Operation::Restart => {
            if target.kind == Kind::Deployment {
                workload::check_restart(&read(parts, job, "read the Deployment", request::get(target)).await?)
                    .map_err(JobState::Failed)?;
            }
            write(parts, job, "restart each pod", workload::restart(target, Timestamp::now())).await
        }
        Operation::Suspend => write(parts, job, "suspend", workload::suspend(target, true)).await,
        Operation::Resume => write(parts, job, "resume", workload::suspend(target, false)).await,
        Operation::Delete(propagation) => remove(parts, job, "delete", delete::delete(target, *propagation)).await,
        Operation::ForceDelete => remove(parts, job, "delete with no grace period", delete::force_delete(target)).await,
        Operation::Evict => remove(parts, job, "evict", delete::evict(target, None)).await,
        Operation::Trigger(name) => {
            let cron_job = read(parts, job, "read the CronJob", request::get(target)).await?;
            let manifest = job::of_cron_job(&cron_job, name).map_err(JobState::Failed)?;
            let create = request::create(Kind::Job, &target.key.namespace, &manifest, false);
            job.reads = Target { kind: Kind::Job, key: ObjectKey::new(&target.key.namespace, name), ..target.clone() };
            write(parts, job, format!("create the Job {name}"), create).await
        }
        Operation::RolloutStatus => long::rollout(target, parts, job).await,
        Operation::RunAgain => long::run_again(target, parts, job).await,
        Operation::History => revisions::history(target, parts, job).await,
        Operation::Rollback(to) => revisions::rollback(target, parts, job, *to).await,
        Operation::Cordon => write(parts, job, "cordon the node", node::cordon(target, true)).await,
        Operation::Uncordon => write(parts, job, "uncordon the node", node::cordon(target, false)).await,
        Operation::Drain(options) => drain::drain(target, options, parts, job).await,
    }
}

/// A step that reads an object as JSON. A read error ends the job: when the server
/// answered, it refused; with no answer, nothing went out of a write.
async fn read<P: Progress>(
    parts: &Parts,
    job: &mut Steps<'_, P>,
    text: &str,
    request: Result<Request<Vec<u8>>, CoreError>,
) -> Result<Json, JobState> {
    job.begin(text);
    let answer = parts.gateway.get::<Json>(request.map_err(JobState::NotSent)?).await.map_err(read_failure)?;
    job.done("read");
    Ok(answer)
}

/// A step that sends one write. The result goes to the outcome for the mark "change pending".
async fn write<P: Progress>(
    parts: &Parts,
    job: &mut Steps<'_, P>,
    text: impl Into<String>,
    request: Result<Request<Vec<u8>>, CoreError>,
) -> Result<(), JobState> {
    let answer = send(parts, job, text, request).await?;
    let written = written(&answer);
    let version = written.as_ref().map_or_else(String::new, |_| version(&answer));
    job.done(format!("the server stored it{version}"));
    job.outcome(Outcome::Written(written));
    Ok(())
}

/// A delete or an eviction: the answer is the object that waits for its end, or a status
/// when it is gone.
async fn remove<P: Progress>(
    parts: &Parts,
    job: &mut Steps<'_, P>,
    text: &str,
    request: Result<Request<Vec<u8>>, CoreError>,
) -> Result<(), JobState> {
    let answer = send(parts, job, text, request).await?;
    let (result, written) = match delete::is_gone(&answer) {
        true => ("the object is gone or goes now", Written::removed(&identity(&job.target))),
        false => ("the object waits for its end", written(&answer)),
    };
    job.done(result);
    job.outcome(Outcome::Written(written));
    Ok(())
}

/// Sends one write after a check of the cancel. A result that is not known gets the read
/// of design rule 6.
async fn send<P: Progress>(
    parts: &Parts,
    job: &mut Steps<'_, P>,
    text: impl Into<String>,
    request: Result<Request<Vec<u8>>, CoreError>,
) -> Result<Json, JobState> {
    if parts.cancel.is_set() {
        return Err(JobState::Cancelled);
    }
    job.begin(text);
    let request = request.map_err(JobState::NotSent)?;
    match parts.gateway.send::<Json>(request).await {
        Ok(answer) => Ok(answer),
        Err(error) => {
            let state = JobState::of_write(error);
            if matches!(state, JobState::Unknown(_)) {
                job.end_step(StepState::Failed);
                read_after(parts, job).await;
            }
            Err(state)
        }
    }
}

/// The read of the object after a write whose result is not known: the user then sees if
/// the write is in effect. The store shows it too, after its next event.
async fn read_after<P: Progress>(parts: &Parts, job: &mut Steps<'_, P>) {
    job.begin("read the object, because the result of the write is not known");
    let result = match request::get(&job.reads) {
        Ok(request) => parts.gateway.get::<Json>(request).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(object) => job.done(format!("the object exists{}", version(&object))),
        Err(error) if error.status() == Some(404) => job.done("the object is gone"),
        Err(error) => job.done(format!("the read failed too: {error}")),
    }
}

/// The state of a job after a read that failed.
fn read_failure(error: CoreError) -> JobState {
    match error.status() {
        Some(_) => JobState::Failed(error),
        None => JobState::NotSent(error),
    }
}

/// The result of a write whose answer is an object.
fn written(answer: &Json) -> Option<Written> {
    let meta: ObjectMeta = serde_json::from_value(answer.get("metadata")?.clone()).ok()?;
    Written::changed(&meta)
}

/// ", version 123" for an object with a resource version.
fn version(object: &Json) -> String {
    let version = object.pointer("/metadata/resourceVersion").and_then(Json::as_str);
    version.map(|version| format!(", version {version}")).unwrap_or_default()
}

/// The metadata of the identity of a target.
fn identity(target: &Target) -> ObjectMeta {
    let namespace = Some(target.key.namespace.to_string()).filter(|namespace| !namespace.is_empty());
    ObjectMeta {
        name: Some(target.key.name.to_string()),
        namespace,
        uid: Some(target.uid.to_string()),
        ..ObjectMeta::default()
    }
}

/// The JSON text of a manifest for the user to copy. "Run again" makes it before its
/// delete: after the delete, an empty text would leave the user with no copy of the Job.
fn manifest_text(manifest: &Json) -> Result<Arc<str>, CoreError> {
    let text =
        serde_json::to_string_pretty(manifest).map_err(|_| CoreError::invalid("the manifest has no JSON text"))?;
    Ok(text.into())
}

#[cfg(test)]
mod tests;
