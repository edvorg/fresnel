use k8s_openapi::api::core::v1::Pod;
use serde_json::{Value as Json, json};

use super::*;
use crate::cluster::{Fate, WriteError};
use crate::error::CoreError;

/// A pod that runs, with a change of its JSON.
fn pod(change: impl FnOnce(&mut Json)) -> Pod {
    let mut pod = json!({"metadata": {"name": "p", "namespace": "shop"},
        "spec": {"containers": [{"name": "c"}]}, "status": {"phase": "Running"}});
    change(&mut pod);
    serde_json::from_value(pod).expect("a pod")
}

fn owned(kind: &'static str) -> impl FnOnce(&mut Json) {
    move |pod| {
        pod["metadata"]["ownerReferences"] =
            json!([{"apiVersion": "apps/v1", "kind": kind, "name": "agent", "uid": "u", "controller": true}]);
    }
}

fn empty_dir(pod: &mut Json) {
    pod["spec"]["volumes"] = json!([{"name": "scratch", "emptyDir": {}}]);
}

const FORCE: DrainOptions = DrainOptions {
    ignore_daemon_sets: false,
    delete_empty_dir_data: false,
    force: true,
    grace_period: None,
    timeout: None,
};

#[test]
fn each_filter_of_kubectl_keeps_blocks_or_evicts_a_pod_in_its_sequence() {
    let none = DrainOptions::default();
    let all = DrainOptions { ignore_daemon_sets: true, delete_empty_dir_data: true, ..FORCE };
    let here = |_: &str| Found::Here;
    let replicated = pod(owned("ReplicaSet"));
    assert_eq!(verdict(&replicated, &none, here), Verdict::Evict(None));

    let daemon = pod(owned("DaemonSet"));
    assert_eq!(verdict(&daemon, &none, here), Verdict::Block(Block::DaemonSet));
    assert_eq!(verdict(&daemon, &all, here), Verdict::Keep("a pod of a DaemonSet"));
    assert_eq!(verdict(&daemon, &none, |_| Found::Missing), Verdict::Block(Block::NoDaemonSet("agent".to_owned())));
    assert_eq!(verdict(&daemon, &FORCE, |_| Found::Missing), Verdict::Evict(Some("its DaemonSet is not there")));
    let unread = verdict(&daemon, &all, |_| Found::Unknown("forbidden".to_owned()));
    assert_eq!(unread, Verdict::Block(Block::Unread("forbidden".to_owned())), "no option removes it");
    let ended = pod(|pod| {
        owned("DaemonSet")(pod);
        pod["status"]["phase"] = "Succeeded".into();
    });
    assert_eq!(verdict(&ended, &none, |_| panic!("a pod that ended needs no DaemonSet")), Verdict::Evict(None));

    let mirror = pod(|pod| {
        owned("Node")(pod);
        pod["metadata"]["annotations"] = json!({MIRROR: "abc"});
    });
    assert!(matches!(verdict(&mirror, &none, here), Verdict::Keep(text) if text.starts_with("a mirror pod")));

    let local = pod(|pod| {
        owned("ReplicaSet")(pod);
        empty_dir(pod);
    });
    assert_eq!(verdict(&local, &FORCE, here), Verdict::Block(Block::LocalData));
    assert_eq!(verdict(&local, &all, here), Verdict::Evict(Some("its emptyDir data goes with it")));

    let lone = pod(|_| {});
    assert_eq!(verdict(&lone, &none, here), Verdict::Block(Block::NoController));
    assert!(matches!(verdict(&lone, &FORCE, here), Verdict::Evict(Some(text)) if text.contains("no controller")));
    // kubectl names only the first filter that refuses a pod.
    let lone_local = pod(empty_dir);
    assert_eq!(verdict(&lone_local, &none, here), Verdict::Block(Block::LocalData));
    let failed = pod(|pod| {
        empty_dir(pod);
        pod["status"]["phase"] = "Failed".into();
    });
    assert_eq!(verdict(&failed, &none, here), Verdict::Evict(None), "a pod that ended passes each filter");
}

#[test]
fn the_text_of_a_refusal_counts_the_pods_and_names_the_first_ones_and_the_flags_follow_kubectl() {
    let keys: Vec<ObjectKey> = (0..7).map(|index| ObjectKey::new("shop", &format!("p{index}"))).collect();
    let pods: Vec<(&ObjectKey, &Block)> = keys.iter().map(|key| (key, &Block::NoController)).collect();
    let text = blocked(&pods);
    assert!(text.starts_with("7 pods block the drain, so no eviction went out. shop/p0: a pod with no controller"));
    assert!(text.ends_with("; and 2 more") && !text.contains("shop/p5"), "{text}");
    assert!(blocked(&pods[..1]).starts_with("1 pod blocks the drain"));

    assert_eq!(DrainOptions::default().flags(), "");
    let options = DrainOptions { grace_period: Some(0), timeout: Some(std::time::Duration::from_secs(60)), ..FORCE };
    assert_eq!(options.flags(), "--force --grace-period=0 --timeout=60s");
}

fn refused(status: u16, causes: &[(&str, &str)]) -> Result<(), WriteError> {
    let causes = causes.iter().map(|(reason, message)| ((*reason).to_owned(), (*message).to_owned())).collect();
    let error = CoreError::answered(status, format!("refused (HTTP {status})")).with_causes(causes);
    Err(WriteError { fate: if status >= 500 { Fate::Unknown } else { Fate::Refused }, error })
}

#[test]
fn an_answer_to_an_eviction_gives_the_next_step_of_kubectl_and_a_budget_by_its_name() {
    assert_eq!(after(Ok(()), false, 0), Next::Wait);
    assert_eq!(after(refused(404, &[]), false, 0), Next::Gone);
    assert_eq!(after(refused(409, &[]), false, 0), Next::Gone, "a new pod has the name");

    let budget = "The disruption budget web-budget needs 1 healthy pods and has 1 currently";
    let blocked = PodState::Blocked { budget: Some("web-budget".to_owned()), reason: budget.to_owned(), attempts: 3 };
    assert_eq!(after(refused(429, &[("DisruptionBudget", budget)]), false, 2), Next::Again(blocked));
    let unnamed = PodState::Blocked { budget: None, reason: "refused (HTTP 429)".to_owned(), attempts: 1 };
    assert_eq!(after(refused(429, &[]), false, 0), Next::Again(unnamed));

    let ending = [("NamespaceTerminating", "namespace shop is being terminated")];
    assert_eq!(after(refused(403, &ending), true, 0), Next::Wait, "a pod with a delete mark goes by itself");
    assert!(matches!(after(refused(403, &ending), false, 0), Next::Again(PodState::Blocked { budget: None, .. })));
    assert!(
        matches!(after(refused(403, &[]), false, 0), Next::End(PodState::Failed(error)) if error.status() == Some(403))
    );
    assert!(matches!(after(refused(500, &[]), false, 0), Next::End(PodState::Unknown(_))), "two budgets: HTTP 500");
}
