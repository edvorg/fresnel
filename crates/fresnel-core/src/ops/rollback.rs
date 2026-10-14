//! The rollback of a workload to a revision, with the algorithm of `kubectl rollout undo`
//! (research 08 sections 4.6 and 4.7). The API has no verb for it.
//!
//! - A Deployment gets a JSON patch: the template of the ReplicaSet of the revision, and
//!   the annotations of that ReplicaSet with some of its own.
//! - A StatefulSet or a DaemonSet gets the data of the ControllerRevision as a strategic
//!   patch: `"$patch": "replace"` in it replaces the full pod template.

use http::Request;
use serde_json::{Map, Value as Json, json};

use super::Target;
use super::history::{self, TEMPLATE_HASH};
use super::request::{self, PatchType};
use crate::error::CoreError;

/// The annotations of a Deployment that stay through a rollback. Each other one comes
/// from the ReplicaSet of the revision.
const KEPT_ANNOTATIONS: [&str; 6] = [
    "kubectl.kubernetes.io/last-applied-configuration",
    history::REVISION,
    "deployment.kubernetes.io/revision-history",
    "deployment.kubernetes.io/desired-replicas",
    "deployment.kubernetes.io/max-replicas",
    "deprecated.deployment.rollback.to",
];

/// What a rollback does.
#[derive(Debug)]
pub(super) enum Plan {
    /// The patch to send. A request is large, and most plans have one.
    Patch(Box<Request<Vec<u8>>>),
    /// The workload has the template of the revision already: no request, as in kubectl.
    Skip(String),
}

/// The rollback of a Deployment to revision `to` (0: the one before the current one).
/// `sets` are the ReplicaSets that the list of its selector gave.
pub(super) fn deployment(target: &Target, deployment: &Json, sets: &[Json], to: i64) -> Result<Plan, CoreError> {
    let set = deployment_revision(deployment, sets, to)?;
    if deployment.pointer("/spec/paused") == Some(&Json::Bool(true)) {
        return Err(CoreError::invalid(
            "you cannot rollback a paused deployment; resume it first with 'kubectl rollout resume' and try again",
        ));
    }
    let mut template = set.pointer("/spec/template").cloned().unwrap_or(Json::Null);
    if history::same_template(Some(&template), deployment.pointer("/spec/template")) {
        return Ok(Plan::Skip(skipped(to)));
    }
    if let Some(labels) = template.pointer_mut("/metadata/labels").and_then(Json::as_object_mut) {
        labels.remove(TEMPLATE_HASH);
    }
    let annotations = |object: &Json| object.pointer("/metadata/annotations").and_then(Json::as_object).cloned();
    let own = annotations(deployment);
    let mut kept: Map<String, Json> = own.iter().flatten().filter(|(key, _)| is_kept(key)).map(cloned).collect();
    kept.extend(annotations(set).iter().flatten().filter(|(key, _)| !is_kept(key)).map(cloned));
    // kubectl sends `replace`. That step fails on an object with no annotations, and `add`
    // does the same as `replace` on a key that exists (RFC 6902).
    let step = if own.is_some() { "replace" } else { "add" };
    let body = json!([
        {"op": "replace", "path": "/spec/template", "value": template},
        {"op": step, "path": "/metadata/annotations", "value": kept},
    ]);
    Ok(Plan::Patch(Box::new(request::patch(target, None, PatchType::Json, &body)?)))
}

/// The rollback of a StatefulSet or a DaemonSet to revision `to` (0: the one before the
/// newest one). `stored` are the ControllerRevisions that the list of its selector gave.
pub(super) fn controller(target: &Target, workload: &Json, stored: &[Json], to: i64) -> Result<Plan, CoreError> {
    let mut owned: Vec<&Json> = history::controlled(workload, stored).collect();
    owned.sort_by_key(|stored| stored.get("revision").and_then(Json::as_i64).unwrap_or(0));
    if to == 0 && owned.len() <= 1 {
        return Err(CoreError::invalid("no last revision to roll back to"));
    }
    let found = match to {
        0 => owned.iter().rev().nth(1).copied(),
        to => owned.iter().copied().find(|stored| stored.get("revision").and_then(Json::as_i64) == Some(to)),
    };
    let data = found.and_then(|stored| stored.get("data")).ok_or_else(|| not_found(to))?;
    let mut now = workload.pointer("/spec/template").cloned().unwrap_or(Json::Null);
    if let Some(map) = now.as_object_mut() {
        map.insert("$patch".to_owned(), json!("replace"));
    }
    if data == &json!({"spec": {"template": now}}) {
        return Ok(Plan::Skip(skipped(to)));
    }
    Ok(Plan::Patch(Box::new(request::patch(target, None, PatchType::Strategic, data)?)))
}

/// The ReplicaSet of revision `to`. For 0, the one with the second highest number, as
/// kubectl finds it. A ReplicaSet whose revision is not a number does not count.
fn deployment_revision<'a>(deployment: &Json, sets: &'a [Json], to: i64) -> Result<&'a Json, CoreError> {
    if to < 0 {
        return Err(not_found(to));
    }
    let numbered = history::controlled(deployment, sets).filter_map(|set| Some((history::set_revision(set)?, set)));
    if to > 0 {
        return numbered.into_iter().find(|(number, _)| *number == to).map(|(_, set)| set).ok_or_else(|| not_found(to));
    }
    let (mut latest, mut previous): (Numbered<'a>, Numbered<'a>) = (None, None);
    for (number, set) in numbered {
        if latest.is_none_or(|(highest, _)| highest < number) {
            previous = latest;
            latest = Some((number, set));
        } else if previous.is_none_or(|(second, _)| second < number) {
            previous = Some((number, set));
        }
    }
    let name = deployment.pointer("/metadata/name").and_then(Json::as_str).unwrap_or_default();
    previous
        .map(|(_, set)| set)
        .ok_or_else(|| CoreError::invalid(format!("no rollout history found for deployment \"{name}\"")))
}

/// A ReplicaSet with its revision.
type Numbered<'a> = Option<(i64, &'a Json)>;

fn is_kept(key: &str) -> bool {
    KEPT_ANNOTATIONS.contains(&key)
}

fn cloned((key, value): (&String, &Json)) -> (String, Json) {
    (key.clone(), value.clone())
}

fn not_found(to: i64) -> CoreError {
    CoreError::invalid(format!("unable to find specified revision {to} in history"))
}

fn skipped(to: i64) -> String {
    format!("skipped rollback (current template already matches revision {to})")
}

#[cfg(test)]
mod tests;
