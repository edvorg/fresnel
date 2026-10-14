//! The revisions of the pod template of a workload, as `kubectl rollout history` finds them
//! (research 08 section 4.5): the ReplicaSets of a Deployment, the ControllerRevisions of a
//! StatefulSet or a DaemonSet.
//!
//! The objects are JSON as the server gave them, not the types of `k8s-openapi`: a type
//! drops each field that it does not know, and a rollback copies a template back into the
//! workload. A field of a newer server would go away with it.

use std::sync::Arc;

use http::Request;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use k8s_openapi::jiff::Timestamp;
use kube::core::Selector;
use serde_json::{Value as Json, json};

use super::Target;
use super::request;
use crate::error::CoreError;
use crate::kinds::Kind;

/// The annotation of a ReplicaSet with the number of its revision.
pub(super) const REVISION: &str = "deployment.kubernetes.io/revision";
/// The annotation with the reason of a change, which `kubectl annotate` or a tool writes.
const CHANGE_CAUSE: &str = "kubernetes.io/change-cause";
/// The label of the pods of a ReplicaSet with the hash of its template. The Deployment
/// template does not have it, so a comparison leaves it out.
pub(super) const TEMPLATE_HASH: &str = "pod-template-hash";

/// One revision of the pod template of a workload.
#[derive(Clone, Debug, PartialEq)]
pub struct Revision {
    /// The number of the revision. A rollback names it.
    pub number: i64,
    /// The name of the ReplicaSet or the ControllerRevision.
    pub name: String,
    /// When the server made it.
    pub created: Option<Timestamp>,
    /// The reason of the change, when a tool wrote one.
    pub change_cause: Option<String>,
    /// The images of the containers of the template.
    pub images: Vec<String>,
    /// True for the template of the workload now.
    pub current: bool,
    /// The pod template, for a comparison of two revisions.
    pub template: Arc<Json>,
}

/// The list of the objects that can hold the revisions of the workload: the ReplicaSets
/// or the ControllerRevisions with its selector, in its namespace. `workload` is the
/// object as the server gave it.
pub(super) fn list(target: &Target, workload: &Json) -> Result<Request<Vec<u8>>, CoreError> {
    let kind = match target.kind {
        Kind::Deployment => Kind::ReplicaSet,
        Kind::StatefulSet | Kind::DaemonSet => Kind::ControllerRevision,
        other => return Err(CoreError::invalid(format!("a {} has no revisions", other.info().name))),
    };
    request::list(kind, &target.key.namespace, &selector(workload)?)
}

/// The revisions of a Deployment from its ReplicaSets, by number. The current one is the
/// oldest ReplicaSet with the template of the Deployment, as kubectl finds its new one.
pub(super) fn of_deployment(deployment: &Json, sets: &[Json]) -> Vec<Revision> {
    let template = deployment.pointer("/spec/template");
    let mut owned: Vec<&Json> = controlled(deployment, sets).collect();
    owned.sort_by_key(|set| (text(set, "/metadata/creationTimestamp"), text(set, "/metadata/name")));
    let current = owned.iter().find(|set| same_template(set.pointer("/spec/template"), template)).copied();
    let mut revisions: Vec<Revision> = owned
        .iter()
        .filter_map(|set| {
            let number = set_revision(set)?;
            let template = set.pointer("/spec/template").cloned().unwrap_or(Json::Null);
            Some(revision(set, number, template, current.is_some_and(|current| std::ptr::eq(current, *set))))
        })
        .collect();
    revisions.sort_by_key(|revision| revision.number);
    revisions
}

/// The revisions of a StatefulSet or a DaemonSet from its ControllerRevisions, by number.
/// The data of each one is the patch `{"spec":{"template":{…,"$patch":"replace"}}}`.
pub(super) fn of_controller(workload: &Json, stored: &[Json]) -> Vec<Revision> {
    let now = workload.pointer("/spec/template");
    let mut revisions: Vec<Revision> = controlled(workload, stored)
        .map(|stored| {
            let number = stored.get("revision").and_then(Json::as_i64).unwrap_or(0);
            let template = template_of_patch(stored.get("data"));
            let current = Some(&template) == now;
            revision(stored, number, template, current)
        })
        .collect();
    revisions.sort_by_key(|revision| revision.number);
    revisions
}

/// The revision of a ReplicaSet. No annotation is revision 0, as in kubectl; a text that
/// is not a number gives none.
pub(super) fn set_revision(set: &Json) -> Option<i64> {
    match set.pointer("/metadata/annotations").and_then(|annotations| annotations.get(REVISION)) {
        None => Some(0),
        Some(text) => text.as_str().and_then(|text| text.parse().ok()),
    }
}

/// The objects that the workload controls: an owner reference with its UID and
/// `controller: true`. An object of an other owner can have the same labels.
pub(super) fn controlled<'a>(workload: &Json, items: &'a [Json]) -> impl Iterator<Item = &'a Json> {
    let uid = workload.pointer("/metadata/uid").and_then(Json::as_str).unwrap_or_default().to_owned();
    items.iter().filter(move |item| {
        let owners = item.pointer("/metadata/ownerReferences").and_then(Json::as_array).into_iter().flatten();
        owners.into_iter().any(|owner| {
            owner.get("controller") == Some(&Json::Bool(true)) && owner.get("uid").and_then(Json::as_str) == Some(&uid)
        })
    })
}

/// True when two pod templates are equal with no label `pod-template-hash`.
pub(super) fn same_template(one: Option<&Json>, other: Option<&Json>) -> bool {
    let without = |template: Option<&Json>| {
        let mut copy = template.cloned().unwrap_or(Json::Null);
        if let Some(labels) = copy.pointer_mut("/metadata/labels").and_then(Json::as_object_mut) {
            labels.remove(TEMPLATE_HASH);
        }
        copy
    };
    without(one) == without(other)
}

/// The pod template in the patch of a ControllerRevision, with no `$patch` key.
fn template_of_patch(data: Option<&Json>) -> Json {
    let mut template = data.and_then(|data| data.pointer("/spec/template")).cloned().unwrap_or(Json::Null);
    if let Some(map) = template.as_object_mut() {
        map.remove("$patch");
    }
    template
}

fn revision(object: &Json, number: i64, template: Json, current: bool) -> Revision {
    let change_cause = object.pointer("/metadata/annotations").and_then(|annotations| annotations.get(CHANGE_CAUSE));
    let containers = template.pointer("/spec/containers").and_then(Json::as_array).into_iter().flatten();
    Revision {
        number,
        name: text(object, "/metadata/name").to_owned(),
        created: text(object, "/metadata/creationTimestamp").parse().ok(),
        change_cause: change_cause.and_then(Json::as_str).map(str::to_owned),
        images: containers.filter_map(|container| container.get("image")?.as_str().map(str::to_owned)).collect(),
        current,
        template: Arc::new(template),
    }
}

/// The label selector of a workload as a text for a list.
fn selector(workload: &Json) -> Result<String, CoreError> {
    let spec = workload.pointer("/spec/selector").cloned().unwrap_or(json!({}));
    let invalid = |_| CoreError::invalid("the workload has a selector that the app cannot read");
    let selector: LabelSelector = serde_json::from_value(spec).map_err(invalid)?;
    let selector =
        Selector::try_from(selector).map_err(|error| CoreError::invalid(format!("the selector: {error}")))?;
    if selector.selects_all() {
        return Err(CoreError::invalid("the workload has an empty selector, which would select each object"));
    }
    Ok(selector.to_string())
}

fn text<'a>(object: &'a Json, pointer: &str) -> &'a str {
    object.pointer(pointer).and_then(Json::as_str).unwrap_or_default()
}

#[cfg(test)]
pub(super) mod tests;
