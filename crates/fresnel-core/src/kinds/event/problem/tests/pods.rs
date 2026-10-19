//! The rules of a pod: its probes, its containers, its sandbox and volumes, and its node.

use super::*;
use crate::fixture::condition;

/// A warning about the pod `name` with this message.
fn said(name: &str, reason: &str, part: &str, message: &str) -> Event {
    let mut event = warning::<Pod>(name, reason, "2026-10-13T17:58:50Z", Some(part));
    event.message = Some(message.to_owned());
    event
}

/// The witness at an other moment.
fn at_moment(witness: Witness, time: &str) -> Witness {
    Witness { at: moment(time), ..witness }
}

#[test]
fn a_failed_readiness_probe_ends_when_its_container_and_its_pod_are_ready_after_the_last_report() {
    let bed = Bed::new();
    let event = said("not-ready", "Unhealthy", "spec.containers{main}", "Readiness probe failed: ");
    let ready = |since: Option<&str>, pod_ready: &str| {
        bed.with::<Pod>("not-ready", |pod| {
            pod["status"]["containerStatuses"][0]["ready"] = true.into();
            let ready = condition(pod, "Ready");
            (ready["status"], ready["reason"]) = (pod_ready.into(), "ReadinessGatesNotReady".into());
            ready["lastTransitionTime"] = since.map_or(Value::Null, Value::from);
        })
    };
    let resolved = state(Resolved(at(LATE)), "the container main is ready");
    assert_eq!(judged(&ready(Some(LATE), "True"), &event), resolved);
    let before = state(Unknown, "the container main is ready since before the last report");
    assert_eq!(judged(&ready(Some(EARLY), "True"), &event), before, "a failure under the threshold of the probe");
    let same = ready(Some("2026-10-13T17:58:50Z"), "True");
    let at_report = state(Unknown, "the container main is ready since before the last report");
    assert_eq!(judged(&same, &event), at_report, "a change at the time of the report proves no end");
    let no_time = state(Unknown, "the container main is ready, and its status gives no time of the change");
    assert_eq!(judged(&ready(None, "True"), &event), no_time);
    let gate = state(Unknown, "the container main is ready, and the pod is not: ReadinessGatesNotReady");
    assert_eq!(judged(&ready(Some(LATE), "False"), &event), gate, "an other cause keeps the pod not ready");

    let no_probe = said("not-ready", "Unhealthy", "spec.containers{main}", "Probe errored: no such host");
    assert_eq!(judged(&ready(Some(LATE), "True"), &no_probe), state(Unknown, "the message names no probe"));
}

/// A liveness probe restarts its container, and its pod is ready again after each restart:
/// only a run of 10 minutes after the last report ends the problem.
#[test]
fn a_failed_liveness_probe_ends_only_after_a_long_run_of_its_restarted_container() {
    let bed = Bed::new();
    let event = said("not-ready", "Unhealthy", "spec.containers{main}", "Liveness probe failed: HTTP 500");
    let restarted = bed.with::<Pod>("not-ready", |pod| {
        let main = &mut pod["status"]["containerStatuses"][0];
        (main["ready"], main["restartCount"]) = (true.into(), 3.into());
        main["state"] = json!({"running": {"startedAt": LATE}});
        let ready = condition(pod, "Ready");
        (ready["status"], ready["lastTransitionTime"]) = ("True".into(), LATE.into());
    });
    let again = "the container main runs again after 3 restarts, for less than 10 min: a crash loop runs it \
                 between its waits";
    let soon = at_moment(restarted.clone(), "2026-10-13T18:19:59Z");
    assert_eq!(judged(&soon, &event), state(Unknown, again), "the pod is ready, but the loop goes on");
    let long = state(Resolved(at(LATE)), "the container main runs again, for 10 min or more");
    assert_eq!(judged(&restarted, &event), long);
    let readiness = said("not-ready", "Unhealthy", "spec.containers{main}", "Readiness probe failed: ");
    assert_eq!(judged(&soon, &readiness), state(Unknown, again), "a ready container that runs again");
    let startup = said("not-ready", "Unhealthy", "spec.containers{main}", "Startup probe failed: ");
    assert_eq!(judged(&soon, &startup), state(Unknown, again));
    let failing = state(Unknown, "the container main runs since before the last report");
    let witness = bed.scene(&fixture::all(), &fixture::all());
    assert_eq!(judged(&witness, &event), failing, "a failure under the threshold restarts nothing");
}

/// The kubelet reports a crash loop during its waits, and starts the container between
/// them: a run that began after the last report is no end before the back-off resets.
#[test]
fn a_container_problem_ends_with_a_long_run_after_the_last_report() {
    let bed = Bed::new();
    let event = warning::<Pod>("crash", "BackOff", "2026-10-13T17:58:53Z", Some("spec.containers{crash}"));
    let in_state = |state: Value, restarts: i32| {
        bed.with::<Pod>("crash", |pod| {
            let crash = &mut pod["status"]["containerStatuses"][0];
            (crash["state"], crash["restartCount"]) = (state, restarts.into());
        })
    };
    let runs = |since: &str, restarts: i32| in_state(json!({"running": {"startedAt": since}}), restarts);
    let again = "the container crash runs again after 5 restarts, for less than 10 min: a crash loop runs it \
                 between its waits";
    let soon = at_moment(runs(LATE, 5), "2026-10-13T18:15:00Z");
    assert_eq!(judged(&soon, &event), state(Unknown, again), "a run between two waits of the loop");
    let long = state(Resolved(at(LATE)), "the container crash runs again, for 10 min or more");
    assert_eq!(judged(&runs(LATE, 5), &event), long, "the back-off reset after 10 min");
    let pulled = state(Resolved(at(LATE)), "the container crash runs");
    assert_eq!(judged(&at_moment(runs(LATE, 0), "2026-10-13T18:15:00Z"), &event), pulled, "a first start");
    let before = state(Unknown, "the container crash runs since before the last report");
    assert_eq!(judged(&runs(EARLY, 5), &event), before);
    let ended =
        |code: i32| in_state(json!({"terminated": {"exitCode": code, "startedAt": LATE, "finishedAt": LATE}}), 0);
    let again = state(Unknown, "the container crash ended, and the kubelet starts it again");
    assert_eq!(judged(&ended(0), &event), again, "a loop of a container that exits with 0");
    assert_eq!(judged(&ended(1), &event), state(Ongoing, "the container crash ended with exit code 1"));
    let waits = in_state(json!({"waiting": {}}), 2);
    assert_eq!(judged(&waits, &event), state(Ongoing, "the container crash waits"), "a wait with no reason");

    let witness = bed.scene(&fixture::all(), &fixture::all());
    let no_part = warning::<Pod>("crash", "BackOff", "2026-10-13T17:58:53Z", None);
    assert_eq!(judged(&witness, &no_part), state(Unknown, "the Event names no container"));
    let other = warning::<Pod>("crash", "Failed", "2026-10-13T17:58:53Z", Some("spec.containers{other}"));
    assert_eq!(judged(&witness, &other), state(Unknown, "the pod gives no state of the container other"));
    let debug = bed.with::<Pod>("crash", |pod| {
        pod["status"]["ephemeralContainerStatuses"] = json!([{"name": "debug", "image": "busybox", "imageID": "",
            "ready": false, "restartCount": 0, "state": {"waiting": {"reason": "ErrImagePull"}}}]);
    });
    let pull = warning::<Pod>("crash", "Failed", "2026-10-13T17:58:53Z", Some("spec.ephemeralContainers{debug}"));
    assert_eq!(judged(&debug, &pull), state(Ongoing, "the container debug waits: ErrImagePull"), "a debug container");
}

/// A good end proves an end only for a container that the kubelet does not start again,
/// and only for a pod that was not stopped.
#[test]
fn a_good_end_proves_an_end_only_for_a_container_that_does_not_start_again() {
    let bed = Bed::new();
    let ended = |pod: &str, name: &str| {
        bed.with::<Pod>(pod, |pod| {
            for list in ["containerStatuses", "initContainerStatuses"] {
                let statuses = pod["status"][list].as_array_mut().into_iter().flatten();
                for status in statuses.filter(|status| status["name"] == name) {
                    status["state"] = json!({"terminated": {"exitCode": 0, "startedAt": LATE, "finishedAt": LATE}});
                }
            }
        })
    };
    let back_off = |pod: &str, part: &str| warning::<Pod>(pod, "BackOff", "2026-10-13T17:58:53Z", Some(part));
    let sidecar = state(Unknown, "the container helper ended, and the kubelet starts it again");
    let helper = back_off("sidecar", "spec.initContainers{helper}");
    assert_eq!(judged(&ended("sidecar", "helper"), &helper), sidecar, "an init container with the policy Always");
    let setup = state(Resolved(at(LATE)), "the container setup ended well");
    let init = back_off("init-crash", "spec.initContainers{setup}");
    assert_eq!(judged(&ended("init-crash", "setup"), &init), setup, "an init container runs one time");
    let worker = state(Resolved(at(LATE)), "the container worker ended well");
    let never = back_off("half-done", "spec.containers{worker}");
    assert_eq!(judged(&ended("half-done", "worker"), &never), worker, "a pod with the policy Never");
    let debug = bed.with::<Pod>("crash", |pod| {
        pod["spec"]["ephemeralContainers"] = json!([{"name": "debug", "image": "busybox"}]);
        pod["status"]["ephemeralContainerStatuses"] = json!([{"name": "debug", "image": "busybox", "imageID": "",
            "ready": false, "restartCount": 0,
            "state": {"terminated": {"exitCode": 0, "startedAt": LATE, "finishedAt": LATE}}}]);
    });
    let pulled = back_off("crash", "spec.ephemeralContainers{debug}");
    let once = state(Resolved(at(LATE)), "the container debug ended well");
    assert_eq!(judged(&debug, &pulled), once, "an ephemeral container runs one time");

    // A pod with the policy Always ends well only when it is stopped, as by a delete: its
    // crash loop can go on in a new pod.
    let stopped = bed.with::<Pod>("crash", |pod| pod["status"]["phase"] = "Succeeded".into());
    let sandbox = warning::<Pod>("crash", "FailedCreatePodSandBox", LATE, None);
    assert_eq!(judged(&stopped, &sandbox), state(Unknown, "the pod was stopped, so no container of it runs"));
}

/// client-go combines the reports of one object and reason with 10 messages in 10 minutes
/// into one Event, which keeps the field path of its first report: it names no container.
#[test]
fn an_event_that_combines_similar_reports_proves_nothing_of_one_container() {
    let bed = Bed::new();
    let runs = bed.with::<Pod>("crash", |pod| {
        let crash = &mut pod["status"]["containerStatuses"][0];
        (crash["state"], crash["restartCount"]) = (json!({"running": {"startedAt": LATE}}), 0.into());
    });
    let pull = said("crash", "Failed", "spec.containers{crash}", "Failed to pull image \"crash\": not found");
    assert_eq!(judged(&runs, &pull).0, Resolved(at(LATE)), "one report of one container");
    let combined = state(Unknown, "the Event combines similar reports, which can be of an other container");
    let message = "(combined from similar events): Failed to pull image \"other\": not found";
    let mixed = said("crash", "Failed", "spec.containers{crash}", message);
    assert_eq!(judged(&runs, &mixed), combined, "the newest report can be of the container other");
    let message = "(combined from similar events): Readiness probe failed: ";
    let probes = said("not-ready", "Unhealthy", "spec.containers{main}", message);
    assert_eq!(judged(&bed.scene(&fixture::all(), &fixture::all()), &probes), combined);
}

#[test]
fn a_pod_with_no_node_no_sandbox_or_no_volumes_gets_them_after_the_last_report() {
    let bed = Bed::new();
    let scheduled = warning::<Pod>("unschedulable", "FailedScheduling", "2026-10-13T17:58:14Z", None);
    let placed = |time: &str| {
        bed.with::<Pod>("unschedulable", |pod| {
            let scheduled = condition(pod, "PodScheduled");
            (scheduled["status"], scheduled["lastTransitionTime"]) = ("True".into(), time.into());
        })
    };
    assert_eq!(judged(&placed(LATE), &scheduled), state(Resolved(at(LATE)), "the pod has a node"));
    let mut no_time = scheduled.clone();
    no_time.last_timestamp = None;
    let unknown = state(Unknown, "the pod has a node, and the Event gives no time of its report");
    assert_eq!(judged(&placed(LATE), &no_time), unknown);

    let sandbox = warning::<Pod>("crash", "FailedCreatePodSandBox", LATE, None);
    let witness = bed.scene(&fixture::all(), &fixture::all());
    let before = state(Unknown, "the pod has its sandbox since before the last report");
    assert_eq!(judged(&witness, &sandbox), before);
    let none = bed.with::<Pod>("crash", |pod| {
        let conditions = pod["status"]["conditions"].as_array_mut().expect("conditions");
        conditions.retain(|condition| condition["type"] != "PodReadyToStartContainers");
    });
    let old_cluster = state(Unknown, "the pod has no condition PodReadyToStartContainers");
    assert_eq!(judged(&none, &sandbox), old_cluster, "a cluster older than Kubernetes 1.29");

    let sandbox_since = |status: &str| {
        bed.with::<Pod>("crash", |pod| {
            let sandbox = condition(pod, "PodReadyToStartContainers");
            (sandbox["status"], sandbox["lastTransitionTime"]) = (status.into(), LATE.into());
        })
    };
    let (waits, made) = (sandbox_since("False"), sandbox_since("True"));
    for (reason, bad, good) in [
        ("FailedCreatePodSandBox", "has no sandbox", "has its sandbox"),
        ("NetworkNotReady", "has no sandbox", "has its sandbox"),
        ("FailedMount", "waits for its volumes and its sandbox", "has its volumes and its sandbox"),
        ("FailedAttachVolume", "waits for its volumes and its sandbox", "has its volumes and its sandbox"),
    ] {
        let event = warning::<Pod>("crash", reason, "2026-10-13T17:58:14Z", None);
        assert_eq!(judged(&waits, &event), (Ongoing, format!("the pod {bad}")), "{reason}");
        assert_eq!(judged(&made, &event), (Resolved(at(LATE)), format!("the pod {good}")), "{reason}");
    }
}

#[test]
fn a_node_that_stopped_its_reports_or_had_a_shortage_is_judged_by_its_conditions() {
    let bed = Bed::new();
    let lost = warning::<Pod>("db-0", "NodeNotReady", "2026-10-13T17:59:00Z", None);
    let ready = |status: &str, since: &str| {
        bed.with::<Node>(NODE, |node| {
            let ready = condition(node, "Ready");
            (ready["status"], ready["lastTransitionTime"]) = (status.into(), since.into());
        })
    };
    let node = format!("the node {NODE}");
    assert_eq!(judged(&ready("Unknown", LATE), &lost), (Ongoing, format!("{node} is not ready")));
    assert_eq!(judged(&ready("True", LATE), &lost), (Resolved(at(LATE)), format!("{node} is ready")));
    let before = (Unknown, format!("{node} is ready since before the last report"));
    assert_eq!(judged(&ready("True", EARLY), &lost), before);
    let no_node = bed.scene(&fixture::all(), &[]);
    assert_eq!(judged(&no_node, &lost), (Unknown, format!("{node} is not in its store")));
    let nodes = bed.source::<Node>(Kind::Node, StoreState::Stopped, &[]);
    let stopped = Witness::new([bed.live(&fixture::all::<Pod>()), nodes], moment(NOW));
    assert_eq!(judged(&stopped, &lost), state(Unknown, "the store of the Nodes is not live: stopped"));
    let waits = warning::<Pod>("unschedulable", "NodeNotReady", "2026-10-13T17:59:00Z", None);
    assert_eq!(judged(&ready("True", LATE), &waits), state(Unknown, "the pod has no node"));

    let shortage = warning::<Node>(NODE, "EvictionThresholdMet", "2026-10-13T17:59:00Z", None);
    let pressure = |status: &str, since: &str| {
        bed.with::<Node>(NODE, |node| {
            let disk = condition(node, "DiskPressure");
            (disk["status"], disk["lastTransitionTime"]) = (status.into(), since.into());
        })
    };
    assert_eq!(judged(&pressure("True", EARLY), &shortage), state(Ongoing, "the node has DiskPressure"));
    assert_eq!(judged(&pressure("False", LATE), &shortage), state(Resolved(at(LATE)), "the node has no pressure"));
    let before = state(Unknown, "the node has no pressure since before the last report");
    assert_eq!(judged(&pressure("False", EARLY), &shortage), before);
    let no_pressure = bed.with::<Node>(NODE, |node| {
        let conditions = node["status"]["conditions"].as_array_mut().expect("conditions");
        conditions.retain(|condition| condition["type"] == "Ready");
    });
    assert_eq!(judged(&no_pressure, &shortage), state(Unknown, "the node has no condition of a pressure"));
}
