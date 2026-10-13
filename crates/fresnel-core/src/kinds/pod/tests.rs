use k8s_openapi::api::core::v1::Pod;

use super::*;
use crate::fixture;
use crate::kinds::ObjectHealth::{Failed, Healthy, Progressing, Warning};

/// The rule of kubectl, on each pod of the scene: the app shows what kubectl showed for
/// the same object.
#[test]
fn the_status_of_each_pod_of_the_fixture_is_what_kubectl_showed() {
    let (pods, lines) = (fixture::all::<Pod>(), fixture::pod_lines());
    assert_eq!((pods.len(), lines.len()), (25, 25), "the pods of the scene and of k3s");
    for (pod, line) in pods.iter().zip(&lines) {
        assert_eq!(pod.metadata.name.as_deref(), Some(line.name.as_str()), "the two lists have one sequence");
        let status = PodStatus::of(pod);
        let shown = (format!("{}/{}", status.ready, status.containers), status.text.as_str(), status.restarts);
        assert_eq!(shown, (line.ready.clone(), line.status.as_str(), line.restarts), "{}", line.name);
        assert_eq!(status.last_restart.is_some(), line.restarted_ago, "the last restart of {}", line.name);
    }
}

/// The scene has one pod for each branch of the rule, so the test above covers them.
#[test]
fn the_fixture_has_a_pod_in_each_state_of_the_rule() {
    let status = |name: &str| PodStatus::of(&fixture::one::<Pod>(name));
    let row = |name: &str| {
        let status = status(name);
        (format!("{} {}/{}", status.text, status.ready, status.containers), status.health)
    };
    let expected = [
        ("web-", "Running 1/1", Healthy),
        ("done", "Completed 0/1", Healthy),
        ("sidecar", "Running 2/2", Healthy),
        ("init-wait", "Init:1/2 0/1", Progressing),
        ("gated", "SchedulingGated 0/1", Progressing),
        ("leaving", "Terminating 1/1", Progressing),
        ("unschedulable", "Pending 0/1", Warning),
        ("not-ready", "Running 0/1", Warning),
        ("half-done", "NotReady 1/2", Warning),
        ("broken-", "ImagePullBackOff 0/1", Failed),
        ("crash", "CrashLoopBackOff 0/1", Failed),
        ("failed", "Error 0/1", Failed),
        ("init-error", "Init:Error 0/1", Failed),
        ("init-crash", "Init:CrashLoopBackOff 0/1", Failed),
        ("no-config", "CreateContainerConfigError 0/1", Failed),
    ];
    for (name, text, health) in expected {
        assert_eq!(row(name), (text.to_owned(), health), "{name}");
    }

    // The last restart is the end of the run before it, as the pod itself tells it.
    let crash = fixture::one::<Pod>("crash");
    let container = &crash.status.as_ref().expect("a status").container_statuses.as_ref().expect("a container")[0];
    let ended = container.last_state.as_ref().and_then(|state| state.terminated.as_ref()?.finished_at.clone());
    let read = status("crash");
    assert!(read.restarts >= 1 && ended.is_some());
    assert_eq!(read.last_restart, ended);
    assert_eq!((status("done").restarts, status("done").last_restart), (0, None));
}

/// The states that the scene cannot hold: they pass in a moment, or need a fault of a node.
#[test]
fn the_rule_reads_the_states_that_no_pod_of_the_scene_is_in() {
    let change = |name: &str, change: &dyn Fn(&mut serde_json::Value)| {
        let mut pod = serde_json::to_value(fixture::one::<Pod>(name)).expect("JSON");
        change(&mut pod);
        let status = PodStatus::of(&serde_json::from_value(pod).expect("a pod"));
        (status.text, status.ready, status.restarts, status.health)
    };
    let set = |pointer: &'static str, value: serde_json::Value| {
        move |pod: &mut serde_json::Value| *pod.pointer_mut(pointer).expect("the field") = value.clone()
    };
    let json = |text: &str| serde_json::from_str::<serde_json::Value>(text).expect("JSON");
    let state = "/status/containerStatuses/0/state";

    // A container that ended with no reason shows its signal, or its exit code.
    let killed = change("failed", &set(state, json(r#"{"terminated":{"exitCode":137,"signal":9}}"#)));
    assert_eq!(killed, ("Signal:9".to_owned(), 0, 0, Failed));
    let ended = change("failed", &set(state, json(r#"{"terminated":{"exitCode":3}}"#)));
    assert_eq!(ended.0, "ExitCode:3");
    let init = "/status/initContainerStatuses/0/state";
    let init_killed = change("init-error", &set(init, json(r#"{"terminated":{"exitCode":2,"signal":0}}"#)));
    assert_eq!(init_killed.0, "Init:ExitCode:2");

    // With two containers that ended, the text is of the first one.
    let two_ended = change("half-done", &|pod| {
        pod["status"]["containerStatuses"][0]["state"] = json(r#"{"terminated":{"exitCode":1,"reason":"Error"}}"#);
        pod["status"]["containerStatuses"][1]["state"] = json(r#"{"terminated":{"exitCode":0,"reason":"Completed"}}"#);
    });
    assert_eq!((two_ended.0.as_str(), two_ended.1), ("Error", 0));

    // A pod that starts: the text is the reason of its container, and that is no fault.
    let creating = change("web-", &|pod| {
        *pod.pointer_mut(state).expect("a state") = json(r#"{"waiting":{"reason":"ContainerCreating"}}"#);
        pod["status"]["containerStatuses"][0]["ready"] = false.into();
        pod["status"]["phase"] = "Pending".into();
    });
    assert_eq!(creating, ("ContainerCreating".to_owned(), 0, 0, Progressing));
    let waits = change(
        "init-wait",
        &set("/status/initContainerStatuses/1/state", json(r#"{"waiting":{"reason":"PodInitializing"}}"#)),
    );
    assert_eq!((waits.0.as_str(), waits.3), ("Init:1/2", Progressing), "this reason of an init container is no text");

    // A node that does not answer: the reason of the status wins, and a delete shows as unknown.
    let lost = change("web-", &|pod| pod["status"]["reason"] = "NodeLost".into());
    assert_eq!((lost.0.as_str(), lost.3), ("NodeLost", Failed));
    let lost_and_deleted = change("leaving", &|pod| pod["status"]["reason"] = "NodeLost".into());
    assert_eq!(lost_and_deleted.0, "Unknown");
    // A pod that ended keeps its text in a delete.
    let done_and_deleted = change("done", &|pod| pod["metadata"]["deletionTimestamp"] = "2026-01-01T00:00:00Z".into());
    assert_eq!((done_and_deleted.0.as_str(), done_and_deleted.3), ("Completed", Healthy));

    // A pod with no status at all, as a bad server can give it.
    let empty = PodStatus::of(&Pod::default());
    assert_eq!((empty.text.as_str(), empty.ready, empty.containers, empty.health), ("", 0, 0, Failed));
}

#[test]
fn only_a_count_of_init_containers_is_progress() {
    assert!(is_init_count("Init:0/1") && is_init_count("Init:12/30"));
    for text in ["Init:Error", "Init:/", "Init:1/", "Init:a/2", "Init:1/2/3", "1/2", "Init:CrashLoopBackOff"] {
        assert!(!is_init_count(text), "{text}");
    }
}
