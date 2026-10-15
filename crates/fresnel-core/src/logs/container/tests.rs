use serde_json::{Value, json};

use super::*;

/// A pod with the containers `init`, `app` and `side`, the statuses of `app` and `side`,
/// and these changes of the JSON.
fn pod(app: Value, side: Value, change: impl FnOnce(&mut Value)) -> Pod {
    let mut pod = json!({
        "metadata": {"name": "web-1", "namespace": "shop"},
        "spec": {
            "initContainers": [{"name": "init"}],
            "containers": [{"name": "app"}, {"name": "side"}],
            "ephemeralContainers": [{"name": "debug"}],
        },
        "status": {
            "phase": "Running",
            "initContainerStatuses": [{"name": "init", "image": "i", "imageID": "", "ready": true, "restartCount": 0,
                "state": {"terminated": {"exitCode": 0, "reason": "Completed", "containerID": "containerd://i1"}}}],
            "containerStatuses": [
                {"name": "app", "image": "a", "imageID": "", "ready": true, "restartCount": 0, "state": app},
                {"name": "side", "image": "s", "imageID": "", "ready": true, "restartCount": 0, "state": side},
            ],
        },
    });
    change(&mut pod);
    serde_json::from_value(pod).expect("a pod")
}

fn running() -> Value {
    json!({"running": {}})
}

/// A pod whose two containers run. The ID of a running instance is in the status, not in
/// its state.
fn plain() -> Pod {
    pod(running(), running(), |pod| {
        pod["status"]["containerStatuses"][0]["containerID"] = json!("containerd://a1");
        pod["status"]["containerStatuses"][1]["containerID"] = json!("containerd://s1");
    })
}

#[test]
fn the_names_have_the_sequence_of_kubectl_and_all_gives_each_one() {
    let pod = plain();
    let all: Vec<String> = chosen(&pod, &Containers::All, None).iter().map(|name| name.to_string()).collect();
    assert_eq!(all, ["init", "app", "side", "debug"]);
    assert_eq!(chosen(&pod, &Containers::Named("other".into()), None), [Arc::from("other")]);
    assert_eq!(chosen(&pod, &Containers::Default, Some(&"side".into())), [Arc::from("side")]);
}

#[test]
fn the_default_container_is_the_one_that_restarts_most_then_the_one_of_kubectl() {
    let quiet = plain();
    assert_eq!(default_of(&[&quiet]).as_deref(), Some("app"), "the first container");

    let annotated = pod(running(), running(), |pod| {
        pod["metadata"]["annotations"] = json!({"kubectl.kubernetes.io/default-container": "side"});
    });
    assert_eq!(default_of(&[&quiet, &annotated]).as_deref(), Some("side"), "the annotation of a pod");

    let restarts = |count: i32| {
        pod(running(), running(), move |pod| {
            pod["status"]["containerStatuses"][1]["restartCount"] = json!(count);
        })
    };
    let (once, twice) = (restarts(1), restarts(2));
    assert_eq!(default_of(&[&annotated, &once]).as_deref(), Some("side"));
    let init = pod(running(), running(), |pod| {
        pod["status"]["initContainerStatuses"][0]["restartCount"] = json!(3);
        pod["status"]["containerStatuses"][1]["restartCount"] = json!(1);
    });
    assert_eq!(default_of(&[&once, &init]).as_deref(), Some("init"), "3 restarts of init and 2 of side");
    assert_eq!(default_of(&[&twice, &init]).as_deref(), Some("init"), "the same count gives the first name");
    assert_eq!(default_of(&[]), None);
}

#[test]
fn a_running_container_serves_its_running_instance() {
    let run = run_of(&plain(), "app", false).expect("the container");
    let served = Served::Instance { id: "containerd://a1".into(), running: true };
    assert_eq!(run, Run { served, starts_again: true, end: None });
    assert_eq!(run_of(&plain(), "nothing", false), None);
}

#[test]
fn a_container_that_waits_after_a_crash_serves_the_instance_that_ended() {
    let pod = pod(json!({"waiting": {"reason": "CrashLoopBackOff"}}), running(), |pod| {
        pod["status"]["containerStatuses"][0]["lastState"] =
            json!({"terminated": {"exitCode": 1, "reason": "Error", "containerID": "containerd://a7"}});
    });
    let run = run_of(&pod, "app", false).expect("the container");
    let served = Served::Instance { id: "containerd://a7".into(), running: false };
    assert_eq!(run, Run { served: served.clone(), starts_again: true, end: Some("Error, exit code 1".to_owned()) });
    assert_eq!(run_of(&pod, "app", true).map(|run| run.served), Some(served), "the earlier instance");
    let none = Served::Nothing("the container has no earlier instance".to_owned());
    assert_eq!(run_of(&pod, "side", true).map(|run| run.served), Some(none));
}

#[test]
fn a_container_that_did_not_start_tells_why() {
    let waiting = json!({"waiting": {"reason": "ImagePullBackOff", "message": "Back-off pulling image \"nope:1\""}});
    let pulling = pod(waiting, json!({"waiting": {"reason": "ContainerCreating"}}), |_| {});
    let text = |pod: &Pod, name: &str| match run_of(pod, name, false).map(|run| run.served) {
        Some(Served::Nothing(text)) => text,
        other => panic!("no log was expected: {other:?}"),
    };
    assert_eq!(
        text(&pulling, "app"),
        "the container did not start: ImagePullBackOff: Back-off pulling image \"nope:1\""
    );
    assert_eq!(text(&pulling, "side"), "the container did not start: ContainerCreating");

    let unscheduled = pod(json!({}), json!({}), |pod| {
        pod["status"] = json!({"phase": "Pending", "conditions": [{"type": "PodScheduled", "status": "False",
            "reason": "Unschedulable", "message": "0/1 nodes are available: 1 Insufficient cpu."}]});
    });
    assert_eq!(text(&unscheduled, "app"), "the pod has no node: 0/1 nodes are available: 1 Insufficient cpu.");
    let pending = pod(json!({}), json!({}), |pod| pod["status"] = json!({"phase": "Pending"}));
    assert_eq!(text(&pending, "debug"), "the container did not start: the pod is Pending");
}

#[test]
fn a_container_starts_again_by_the_policy_of_its_pod() {
    let ended = |code: i32, policy: &str, phase: &str| {
        let state = json!({"terminated": {"exitCode": code, "reason": "R", "containerID": "containerd://a2"}});
        let pod = pod(state, running(), |pod| {
            pod["spec"]["restartPolicy"] = json!(policy);
            pod["status"]["phase"] = json!(phase);
        });
        run_of(&pod, "app", false).expect("the container")
    };
    let again = |code, policy, phase| ended(code, policy, phase).starts_again;
    assert_eq!(
        [again(0, "Always", "Running"), again(0, "OnFailure", "Running"), again(1, "OnFailure", "Running")],
        [true, false, true]
    );
    assert!(!again(1, "Never", "Running"));
    assert!(!again(1, "Always", "Failed"), "a pod that ended starts nothing");
    let done = ended(0, "Never", "Succeeded");
    let served = Served::Instance { id: "containerd://a2".into(), running: false };
    assert_eq!(done, Run { served, starts_again: false, end: Some("R, exit code 0".to_owned()) });
    let init = run_of(&plain(), "init", false).expect("the init container");
    assert_eq!((init.starts_again, init.end.as_deref()), (false, Some("Completed, exit code 0")));
}
