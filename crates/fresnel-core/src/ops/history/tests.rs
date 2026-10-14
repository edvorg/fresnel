use serde_json::{Value as Json, json};

use super::super::request::tests::{parts, target};
use super::*;

/// A pod template with one container of this image.
pub(in crate::ops) fn template(image: &str, hash: Option<&str>) -> Json {
    let mut labels = json!({"app": "web"});
    if let Some(hash) = hash {
        labels[TEMPLATE_HASH] = json!(hash);
    }
    json!({"metadata": {"labels": labels, "creationTimestamp": null},
        "spec": {"containers": [{"name": "web", "image": image}]}})
}

/// A Deployment with the UID `dep` and a template of this image.
pub(in crate::ops) fn deployment(image: &str, annotations: Json) -> Json {
    json!({"metadata": {"name": "web", "namespace": "shop", "uid": "dep", "annotations": annotations},
        "spec": {"selector": {"matchLabels": {"app": "web"}}, "template": template(image, None)}})
}

/// A ReplicaSet of the Deployment `dep` (or of `owner`) with this revision.
pub(in crate::ops) fn set(name: &str, revision: &str, image: &str, created: &str, owner: &str) -> Json {
    json!({"metadata": {"name": name, "namespace": "shop", "creationTimestamp": created,
            "annotations": {REVISION: revision, "kubernetes.io/change-cause": format!("set {image}")},
            "ownerReferences": [{"kind": "Deployment", "name": "web", "uid": owner, "controller": true}]},
        "spec": {"template": template(image, Some(name))}})
}

/// A ControllerRevision of the workload `dep` with this revision and image.
pub(in crate::ops) fn stored(name: &str, revision: i64, image: &str) -> Json {
    let mut patch = template(image, None);
    patch["$patch"] = json!("replace");
    json!({"metadata": {"name": name, "creationTimestamp": "2026-10-01T00:00:00Z",
            "ownerReferences": [{"kind": "StatefulSet", "uid": "dep", "controller": true}]},
        "revision": revision, "data": {"spec": {"template": patch}}})
}

#[test]
fn the_revisions_of_a_deployment_come_from_its_own_replica_sets_with_the_current_one() {
    let sets = [
        set("web-3", "3", "web:3", "2026-10-03T00:00:00Z", "dep"),
        set("web-1", "1", "web:1", "2026-10-01T00:00:00Z", "dep"),
        set("web-2", "2", "web:2", "2026-10-02T00:00:00Z", "dep"),
        set("other", "9", "web:2", "2026-09-01T00:00:00Z", "an-other-deployment"),
        set("broken", "not a number", "web:x", "2026-10-04T00:00:00Z", "dep"),
    ];
    let revisions = of_deployment(&deployment("web:2", json!({})), &sets);
    let seen: Vec<(i64, &str, bool)> =
        revisions.iter().map(|revision| (revision.number, revision.name.as_str(), revision.current)).collect();
    assert_eq!(seen, [(1, "web-1", false), (2, "web-2", true), (3, "web-3", false)]);
    let second = &revisions[1];
    assert_eq!((second.images.clone(), second.change_cause.as_deref()), (vec!["web:2".to_owned()], Some("set web:2")));
    assert_eq!(second.created, Some("2026-10-02T00:00:00Z".parse().expect("a time")));
    assert_eq!(*second.template, template("web:2", Some("web-2")));

    // Two ReplicaSets with the template of the Deployment: the oldest is the current one.
    let twins = [
        set("late", "5", "web:2", "2026-10-05T00:00:00Z", "dep"),
        set("early", "4", "web:2", "2026-10-04T00:00:00Z", "dep"),
    ];
    let revisions = of_deployment(&deployment("web:2", json!({})), &twins);
    let current: Vec<&str> =
        revisions.iter().filter(|revision| revision.current).map(|revision| revision.name.as_str()).collect();
    assert_eq!(current, ["early"]);
}

#[test]
fn the_revisions_of_a_stateful_set_come_from_its_controller_revisions() {
    let mut foreign = stored("db-x", 7, "db:1");
    foreign["metadata"]["ownerReferences"][0]["uid"] = json!("other");
    let all = [stored("db-2", 2, "db:2"), stored("db-1", 1, "db:1"), foreign];
    let mut workload = deployment("db:2", json!({}));
    workload["metadata"]["name"] = json!("db");
    let revisions = of_controller(&workload, &all);
    let seen: Vec<(i64, bool)> = revisions.iter().map(|revision| (revision.number, revision.current)).collect();
    assert_eq!(seen, [(1, false), (2, true)]);
    assert_eq!(*revisions[0].template, template("db:1", None), "the template has no `$patch`");
    assert_eq!(revisions[0].change_cause, None);
}

#[test]
fn the_list_of_the_revisions_has_the_selector_of_the_workload() {
    let workload = json!({"spec": {"selector": {"matchLabels": {"app": "web"},
        "matchExpressions": [{"key": "tier", "operator": "In", "values": ["a"]}]}}});
    let (method, url, _, _) = parts(&list(&target(Kind::Deployment, "shop", "web"), &workload).expect("a request"));
    assert_eq!(method, "GET");
    assert_eq!(url, "/apis/apps/v1/namespaces/shop/replicasets?&labelSelector=app%3Dweb%2Ctier+in+%28a%29");
    let daemon = parts(&list(&target(Kind::DaemonSet, "shop", "agent"), &workload).expect("a request")).1;
    assert!(daemon.starts_with("/apis/apps/v1/namespaces/shop/controllerrevisions?&labelSelector="));
    let error = list(&target(Kind::Deployment, "shop", "web"), &json!({"spec": {"selector": {}}})).expect_err("empty");
    assert_eq!(error.to_string(), "the workload has an empty selector, which would select each object");
    assert!(list(&target(Kind::Job, "shop", "once"), &workload).is_err(), "a Job has no revisions");
}

#[test]
fn a_replica_set_with_no_revision_annotation_is_revision_zero() {
    assert_eq!(set_revision(&json!({"metadata": {}})), Some(0));
    assert_eq!(set_revision(&json!({"metadata": {"annotations": {REVISION: "12"}}})), Some(12));
    assert_eq!(set_revision(&json!({"metadata": {"annotations": {REVISION: "x"}}})), None);
    assert!(same_template(Some(&template("a", Some("h1"))), Some(&template("a", None))));
    assert!(!same_template(Some(&template("a", None)), Some(&template("b", None))));
}
