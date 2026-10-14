use serde::de::DeserializeOwned;
use serde_json::{Value as Json, json};

use super::*;

fn object<K: DeserializeOwned>(value: Json) -> K {
    serde_json::from_value(value).expect("an object")
}

fn deployment(generation: i64, replicas: i32, status: Json) -> Deployment {
    object(json!({"metadata": {"name": "web", "generation": generation}, "spec": {"replicas": replicas,
        "selector": {}, "template": {}}, "status": status}))
}

#[test]
fn a_deployment_follows_each_step_of_kubectl() {
    let waiting = |text: &str| Rollout::Waiting(format!("Waiting for deployment \"web\" rollout to finish: {text}..."));
    let unseen = deployment(2, 3, json!({"observedGeneration": 1}));
    assert_eq!(unseen.rollout(), Rollout::Waiting("Waiting for deployment spec update to be observed...".into()));

    let late = json!({"observedGeneration": 2, "conditions": [{"type": "Progressing", "status": "False",
        "reason": "ProgressDeadlineExceeded"}], "updatedReplicas": 1});
    assert_eq!(
        deployment(2, 3, late).rollout(),
        Rollout::Failed("deployment \"web\" exceeded its progress deadline".into())
    );

    let new = json!({"observedGeneration": 2, "replicas": 4, "updatedReplicas": 1});
    assert_eq!(deployment(2, 3, new).rollout(), waiting("1 out of 3 new replicas have been updated"));
    let old = json!({"observedGeneration": 2, "replicas": 4, "updatedReplicas": 3});
    assert_eq!(deployment(2, 3, old).rollout(), waiting("1 old replicas are pending termination"));
    let starting = json!({"observedGeneration": 2, "replicas": 3, "updatedReplicas": 3, "availableReplicas": 2});
    assert_eq!(deployment(2, 3, starting).rollout(), waiting("2 of 3 updated replicas are available"));
    let done = json!({"observedGeneration": 2, "replicas": 3, "updatedReplicas": 3, "availableReplicas": 3});
    let complete = deployment(2, 3, done).rollout();
    assert_eq!(complete, Rollout::Complete("deployment \"web\" successfully rolled out".into()));
    assert!(complete.is_final() && !waiting("x").is_final());
}

#[test]
fn a_daemon_set_counts_its_scheduled_pods_and_needs_a_rolling_update() {
    let daemon = |strategy: &str, status: Json| -> DaemonSet {
        object(json!({"metadata": {"name": "agent", "generation": 1}, "spec": {"selector": {}, "template": {},
            "updateStrategy": {"type": strategy}}, "status": status}))
    };
    let base = json!({"observedGeneration": 1, "currentNumberScheduled": 2, "numberMisscheduled": 0,
        "numberReady": 1, "desiredNumberScheduled": 2});
    let with = |extra: Json| {
        let mut status = base.clone();
        status.as_object_mut().expect("a map").extend(extra.as_object().expect("a map").clone());
        status
    };
    let text = |rest: &str| format!("Waiting for daemon set \"agent\" rollout to finish: {rest}...");
    let updating = daemon("RollingUpdate", with(json!({"updatedNumberScheduled": 1})));
    assert_eq!(updating.rollout(), Rollout::Waiting(text("1 out of 2 new pods have been updated")));
    let starting = daemon("RollingUpdate", with(json!({"updatedNumberScheduled": 2, "numberAvailable": 1})));
    assert_eq!(starting.rollout(), Rollout::Waiting(text("1 of 2 updated pods are available")));
    let done = daemon("RollingUpdate", with(json!({"updatedNumberScheduled": 2, "numberAvailable": 2})));
    assert_eq!(done.rollout(), Rollout::Complete("daemon set \"agent\" successfully rolled out".into()));
    let manual = daemon("OnDelete", with(json!({})));
    assert_eq!(
        manual.rollout(),
        Rollout::Failed("rollout status is only available for RollingUpdate strategy type".into())
    );
}

#[test]
fn a_stateful_set_waits_for_ready_pods_and_its_partition() {
    let set = |rolling: Json, status: Json| -> StatefulSet {
        object(json!({"metadata": {"name": "db", "generation": 3}, "spec": {"replicas": 3, "selector": {},
            "template": {}, "serviceName": "db", "updateStrategy": {"type": "RollingUpdate", "rollingUpdate": rolling}},
            "status": status}))
    };
    let unseen = set(json!({"partition": 0}), json!({"replicas": 3, "observedGeneration": 2}));
    assert_eq!(unseen.rollout(), Rollout::Waiting("Waiting for statefulset spec update to be observed...".into()));
    let never = set(json!({"partition": 0}), json!({"replicas": 3}));
    assert!(matches!(never.rollout(), Rollout::Waiting(_)), "an observed generation of 0 is not seen");
    let unready = set(json!({"partition": 0}), json!({"replicas": 3, "observedGeneration": 3, "readyReplicas": 1}));
    assert_eq!(unready.rollout(), Rollout::Waiting("Waiting for 2 pods to be ready...".into()));
    let ready = json!({"replicas": 3, "observedGeneration": 3, "readyReplicas": 3, "updatedReplicas": 1});
    let partitioned = set(json!({"partition": 1}), ready.clone());
    let text = "Waiting for partitioned roll out to finish: 1 out of 2 new pods have been updated...";
    assert_eq!(partitioned.rollout(), Rollout::Waiting(text.into()));
    let mut two = ready.clone();
    two["updatedReplicas"] = json!(2);
    let text = "partitioned roll out complete: 2 new pods have been updated...";
    assert_eq!(set(json!({"partition": 1}), two).rollout(), Rollout::Complete(text.into()));

    // With no `rollingUpdate`, as only a hand-made object has, the revisions decide.
    let revisions = |current: &str| {
        let status = json!({"replicas": 3, "observedGeneration": 3, "readyReplicas": 3, "updatedReplicas": 1,
            "currentReplicas": 3, "currentRevision": current, "updateRevision": "db-2"});
        let mut set: StatefulSet = object(json!({"metadata": {"name": "db", "generation": 3}, "spec": {"replicas": 3,
            "selector": {}, "template": {}, "serviceName": "db"}, "status": status}));
        set.spec.as_mut().expect("a spec").update_strategy = None;
        set.rollout()
    };
    let text = "waiting for statefulset rolling update to complete 1 pods at revision db-2...";
    assert_eq!(revisions("db-1"), Rollout::Waiting(text.into()));
    assert_eq!(
        revisions("db-2"),
        Rollout::Complete("statefulset rolling update complete 3 pods at revision db-2...".into())
    );
}
