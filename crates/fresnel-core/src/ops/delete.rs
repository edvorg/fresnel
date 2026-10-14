//! Delete, force delete and evict (research 08 sections 4.8 and 4.9).
//!
//! Each one sends the UID of the object that the user sees as a precondition, so a new
//! object of the same name stays (HTTP 409). Each delete sends a propagation policy: with
//! none, the server keeps the pods of a Job or of a ReplicationController.

use http::Request;
use kube::core::params::{DeleteParams, Preconditions, PropagationPolicy};
use serde_json::{Value as Json, json};

use super::request::{self, collection};
use super::{Propagation, Target};
use crate::error::CoreError;

/// The delete of the object with a propagation policy. kubectl sends no grace period, so
/// a pod gets its own one.
pub(super) fn delete(target: &Target, propagation: Propagation) -> Result<Request<Vec<u8>>, CoreError> {
    send_delete(target, params(target, propagation)?)
}

/// `kubectl delete --force`: a grace period of 0, so the server removes the object at once
/// and does not wait for the kubelet. The process of a pod can go on to run on its node.
pub(super) fn force_delete(target: &Target) -> Result<Request<Vec<u8>>, CoreError> {
    send_delete(target, params(target, Propagation::Background)?.grace_period(0))
}

/// The eviction of a pod: a `policy/v1` Eviction for the subresource `eviction`. kube puts
/// the delete options of its helper `Api::evict` under a key that the server ignores, so
/// the body comes from here. The name in the body must be the name in the path.
pub(super) fn evict(target: &Target) -> Result<Request<Vec<u8>>, CoreError> {
    let uid = uid(target)?;
    let (namespace, name) = (&*target.key.namespace, &*target.key.name);
    let body = json!({
        "apiVersion": "policy/v1",
        "kind": "Eviction",
        "metadata": {"name": name, "namespace": namespace},
        "deleteOptions": {"preconditions": {"uid": uid}},
    });
    let collection = collection(target.kind, namespace)?;
    let created = collection.create_subresource("eviction", name, &Default::default(), request::to_bytes(&body)?);
    request::built("the eviction", created)
}

/// True when the answer of a delete is a `Status`: the object is gone. Else the answer is
/// the object, which waits for a finalizer or for the end of its pod.
pub(super) fn is_gone(answer: &Json) -> bool {
    answer.get("kind").and_then(Json::as_str) == Some("Status")
}

fn send_delete(target: &Target, params: DeleteParams) -> Result<Request<Vec<u8>>, CoreError> {
    request::built("the delete", collection(target.kind, &target.key.namespace)?.delete(&target.key.name, &params))
}

fn params(target: &Target, propagation: Propagation) -> Result<DeleteParams, CoreError> {
    let preconditions = Preconditions { uid: Some(uid(target)?.to_owned()), resource_version: None };
    let policy = match propagation {
        Propagation::Background => PropagationPolicy::Background,
        Propagation::Foreground => PropagationPolicy::Foreground,
        Propagation::Orphan => PropagationPolicy::Orphan,
    };
    Ok(DeleteParams { propagation_policy: Some(policy), ..DeleteParams::default() }.preconditions(preconditions))
}

/// A delete with no UID could remove an other object of the name.
fn uid(target: &Target) -> Result<&str, CoreError> {
    Some(&*target.uid).filter(|uid| !uid.is_empty()).ok_or_else(|| {
        CoreError::invalid(format!("{} has no UID, so a delete could reach an other object", target.key.name))
    })
}

#[cfg(test)]
mod tests {
    use super::super::request::tests::{parts, target};
    use super::*;
    use crate::kinds::Kind;

    #[test]
    fn a_delete_sends_its_policy_and_the_uid_in_the_body() {
        let (method, url, content, body) =
            parts(&delete(&target(Kind::Job, "shop", "once"), Propagation::Background).expect("a request"));
        assert_eq!((method.as_str(), url.as_str()), ("DELETE", "/apis/batch/v1/namespaces/shop/jobs/once?"));
        assert_eq!(content, "application/json");
        assert_eq!(body, json!({"propagationPolicy": "Background", "preconditions": {"uid": "uid-1"}}));
        let orphan = parts(&delete(&target(Kind::Namespace, "", "shop"), Propagation::Orphan).expect("a request"));
        assert_eq!(orphan.1, "/api/v1/namespaces/shop?");
        assert_eq!(orphan.3, json!({"propagationPolicy": "Orphan", "preconditions": {"uid": "uid-1"}}));
        let foreground =
            parts(&delete(&target(Kind::Pod, "shop", "web-1"), Propagation::Foreground).expect("a request"));
        assert_eq!(foreground.3["propagationPolicy"], "Foreground");
    }

    #[test]
    fn a_force_delete_has_a_grace_period_of_zero() {
        let (method, url, _, body) = parts(&force_delete(&target(Kind::Pod, "shop", "web-1")).expect("a request"));
        assert_eq!((method.as_str(), url.as_str()), ("DELETE", "/api/v1/namespaces/shop/pods/web-1?"));
        let expected =
            json!({"gracePeriodSeconds": 0, "propagationPolicy": "Background", "preconditions": {"uid": "uid-1"}});
        assert_eq!(body, expected);
    }

    #[test]
    fn an_eviction_is_a_policy_v1_object_with_the_name_of_the_path() {
        let (method, url, content, body) = parts(&evict(&target(Kind::Pod, "shop", "web-1")).expect("a request"));
        assert_eq!((method.as_str(), url.as_str()), ("POST", "/api/v1/namespaces/shop/pods/web-1/eviction?"));
        assert_eq!(content, "application/json");
        let expected = json!({
            "apiVersion": "policy/v1",
            "kind": "Eviction",
            "metadata": {"name": "web-1", "namespace": "shop"},
            "deleteOptions": {"preconditions": {"uid": "uid-1"}},
        });
        assert_eq!(body, expected);
    }

    #[test]
    fn no_delete_goes_without_a_uid_and_a_status_tells_a_delete_that_completed() {
        let mut nameless = target(Kind::Pod, "shop", "web-1");
        nameless.uid = "".into();
        let error = force_delete(&nameless).expect_err("no UID");
        assert_eq!(error.to_string(), "web-1 has no UID, so a delete could reach an other object");
        assert!(evict(&nameless).is_err() && delete(&nameless, Propagation::Background).is_err());
        assert!(is_gone(&json!({"kind": "Status", "status": "Success"})));
        assert!(!is_gone(&json!({"kind": "Pod", "metadata": {"deletionTimestamp": "2026-10-12T20:00:00Z"}})));
    }
}
