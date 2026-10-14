//! The steps that wait for a store: the rollout status (plan section 7.3: a task with steps
//! and a cancel) and "run again" of a Job, which waits for the delete of the old Job.
//!
//! A wait reads the snapshots of the store of the kind, which the watch keeps current, so
//! it sends no request. A cancel ends a wait at once.

use std::pin::pin;

use futures_util::future::{Either, select};
use k8s_openapi::api::batch::v1::Job;
use serde_json::Value as Json;

use super::super::rollout::{Rollout, Rolls};
use super::super::{Propagation, Target, delete, job, request};
use super::{JobState, Outcome, Parts, Progress, Steps, manifest_text, read, read_failure, send, write};
use crate::error::CoreError;
use crate::kinds::{Kind, Object};
use crate::published::Reader;
use crate::store::{Snapshot, StoreState};

/// What a snapshot holds of the object of a target.
enum Seen<'a, K> {
    /// The object, with the UID of the target.
    Object(&'a K),
    /// The store had its complete list, and the object of the target is not in it.
    Gone,
    /// The store did not list its objects yet.
    NotYet,
    /// The store follows no object now, so no event comes: the app stopped it (its cluster
    /// is not connected), or the server refused its list or does not have the kind. A
    /// wait for an event would not end.
    Blocked(StoreState),
}

fn look<'a, K: Object>(snapshot: &'a Snapshot<K>, target: &Target) -> Seen<'a, K> {
    let same = |object: &K| target.uid.is_empty() || object.meta().uid.as_deref() == Some(&*target.uid);
    if matches!(snapshot.state, StoreState::Stopped | StoreState::Forbidden { .. } | StoreState::NotSupported { .. }) {
        return Seen::Blocked(snapshot.state.clone());
    }
    match snapshot.items.get(&target.key) {
        Some(object) if same(object) => Seen::Object(object),
        // An other object with the name of the old one is not the object of the job.
        Some(_) => Seen::Gone,
        None if snapshot.listed => Seen::Gone,
        None => Seen::NotYet,
    }
}

/// Follows the rollout of a workload in its store until it completes, fails or the user
/// cancels it. The outcome has each new state.
pub(super) async fn rollout<P: Progress>(
    target: &Target,
    parts: &Parts,
    job: &mut Steps<'_, P>,
) -> Result<(), JobState> {
    match target.kind {
        Kind::Deployment => follow(target, parts.deployments.clone(), parts, job).await,
        Kind::StatefulSet => follow(target, parts.stateful_sets.clone(), parts, job).await,
        Kind::DaemonSet => follow(target, parts.daemon_sets.clone(), parts, job).await,
        other => Err(JobState::NotSent(CoreError::invalid(format!("a {} has no rollout", other.info().name)))),
    }
}

async fn follow<K: Rolls, P: Progress>(
    target: &Target,
    reader: Option<Reader<Snapshot<K>>>,
    parts: &Parts,
    job: &mut Steps<'_, P>,
) -> Result<(), JobState> {
    let mut reader = reader.ok_or_else(|| no_store(K::ID))?;
    let mut cancel = parts.cancel.clone();
    job.begin("follow the rollout");
    loop {
        let rollout = match look(&reader.read(), target) {
            Seen::Object(object) => Some(object.rollout()),
            Seen::Gone => return Err(JobState::Failed(CoreError::invalid("the object is gone"))),
            Seen::Blocked(state) => return Err(blocked(K::ID, &state)),
            Seen::NotYet => None,
        };
        if let Some(rollout) = rollout {
            if job.report.outcome != Outcome::Rollout(rollout.clone()) {
                job.outcome(Outcome::Rollout(rollout.clone()));
            }
            match rollout {
                Rollout::Complete(text) => {
                    job.done(text);
                    return Ok(());
                }
                Rollout::Failed(text) => return Err(JobState::Failed(CoreError::invalid(text))),
                Rollout::Waiting(_) => {}
            }
        }
        if let Either::Right(_) = select(pin!(reader.changed()), pin!(cancel.until_set())).await {
            return Err(JobState::Cancelled);
        }
    }
}

/// Deletes a Job and creates it again from its manifest (research 08 section 5.3). The
/// selector and the pod template of a Job do not change, so the API has no verb for it.
///
/// The server checks the new manifest in a dry run before the delete. After the delete,
/// the outcome holds the manifest: when the create fails or the user cancels the wait,
/// the user can copy it. Each check that can fail comes before the delete: after it, a
/// failure leaves the user with no Job.
pub(super) async fn run_again<P: Progress>(
    target: &Target,
    parts: &Parts,
    job: &mut Steps<'_, P>,
) -> Result<(), JobState> {
    let jobs = parts.jobs.clone().ok_or_else(|| no_store(Kind::Job))?;
    if let Seen::Blocked(state) = look(&jobs.read(), target) {
        return Err(blocked(Kind::Job, &state));
    }
    let old = read(parts, job, "read the Job", request::get(target)).await?;
    let uid = old.pointer("/metadata/uid").and_then(Json::as_str).unwrap_or_default();
    if uid.is_empty() || (!target.uid.is_empty() && uid != &*target.uid) {
        return Err(JobState::Failed(CoreError::invalid(
            "the Job of this name is an other Job now: it has an other UID",
        )));
    }
    let owners = old.pointer("/metadata/ownerReferences").and_then(Json::as_array).cloned().unwrap_or_default();
    let mut kept = Vec::new();
    for owner in &owners {
        if owner_exists(target, owner, parts, job).await? {
            kept.push(owner.get("uid").cloned());
        }
    }
    let manifest = job::again(&old, |owner| kept.contains(&owner.get("uid").cloned())).map_err(JobState::Failed)?;
    let text = manifest_text(&manifest).map_err(JobState::Failed)?;
    let namespace = &target.key.namespace;
    check(parts, job, request::create(Kind::Job, namespace, &job::for_check(&manifest), true)).await?;
    job.outcome(Outcome::Manifest(text));

    let identity = Target { uid: uid.into(), ..target.clone() };
    send(parts, job, "delete the old Job and its pods", delete::delete(&identity, Propagation::Background)).await?;
    job.done("the server deletes it");
    wait_gone(&identity, jobs, parts, job).await?;
    write(parts, job, "create the new Job", request::create(Kind::Job, namespace, &manifest, false)).await
}

/// The dry run of the new Job. The server stores nothing, so it fails as a read fails: no
/// result of it is unknown.
async fn check<P: Progress>(
    parts: &Parts,
    job: &mut Steps<'_, P>,
    request: Result<http::Request<Vec<u8>>, CoreError>,
) -> Result<(), JobState> {
    job.begin("check the new Job with a dry run");
    let request = request.map_err(JobState::NotSent)?;
    parts.gateway.send::<Json>(request).await.map_err(|error| read_failure(error.error))?;
    job.done("the server accepts it");
    Ok(())
}

/// True when the owner of an owner reference exists with its UID. The garbage collector
/// deletes a Job whose owner is gone, so the new Job keeps only such an owner.
async fn owner_exists<P: Progress>(
    target: &Target,
    owner: &Json,
    parts: &Parts,
    job: &mut Steps<'_, P>,
) -> Result<bool, JobState> {
    let field = |key: &str| owner.get(key).and_then(Json::as_str).unwrap_or_default();
    let (name, uid) = (field("name"), field("uid"));
    let kind = Kind::named(field("kind")).filter(|kind| api_version(*kind) == field("apiVersion"));
    job.begin(format!("look for the owner {} {name}", field("kind")));
    let Some(kind) = kind.filter(|_| !name.is_empty()) else {
        job.done("the app does not know this kind: the new Job has no owner");
        return Ok(false);
    };
    let namespace = if kind.info().namespaced { &*target.key.namespace } else { "" };
    let request = request::get_named(kind, namespace, name).map_err(JobState::NotSent)?;
    match parts.gateway.get::<Json>(request).await {
        Ok(found) if found.pointer("/metadata/uid").and_then(Json::as_str) == Some(uid) => {
            job.done("it exists: the new Job keeps it as its owner");
            Ok(true)
        }
        Ok(_) => {
            job.done("an other object has its name now: the new Job has no owner");
            Ok(false)
        }
        Err(error) if error.status() == Some(404) => {
            job.done("it is gone: the new Job has no owner");
            Ok(false)
        }
        Err(error) => Err(read_failure(error)),
    }
}

/// Waits until the store of the Jobs has no object with the UID of the old Job.
async fn wait_gone<P: Progress>(
    target: &Target,
    mut reader: Reader<Snapshot<Job>>,
    parts: &Parts,
    job: &mut Steps<'_, P>,
) -> Result<(), JobState> {
    let mut cancel = parts.cancel.clone();
    job.begin("wait until the old Job is gone");
    loop {
        match look(&reader.read(), target) {
            Seen::Gone => break,
            Seen::Blocked(state) => return Err(blocked(Kind::Job, &state)),
            Seen::Object(_) | Seen::NotYet => {}
        }
        if let Either::Right(_) = select(pin!(reader.changed()), pin!(cancel.until_set())).await {
            return Err(JobState::Cancelled);
        }
    }
    job.done("the old Job is gone");
    Ok(())
}

/// The `apiVersion` of the objects of a kind.
fn api_version(kind: Kind) -> String {
    let info = kind.info();
    if info.group.is_empty() { info.version.to_owned() } else { format!("{}/{}", info.group, info.version) }
}

/// The end of a wait on a store that follows no object (`Seen::Blocked`).
fn blocked(kind: Kind, state: &StoreState) -> JobState {
    let name = kind.info().name;
    let text = match state {
        StoreState::Stopped => format!("the store of the kind {name} stopped: the cluster is not connected now"),
        other => format!("the store of the kind {name} follows no object: {other}"),
    };
    JobState::Failed(CoreError::invalid(text))
}

fn no_store(kind: Kind) -> JobState {
    JobState::NotSent(CoreError::invalid(format!("the cluster has no store of the kind {}", kind.info().name)))
}
