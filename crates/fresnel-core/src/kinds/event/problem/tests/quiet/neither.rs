//! Each fact of a pod that proves neither an end nor that the problem goes on gets the
//! evidence of the pace of the reports.

use super::*;

/// A change of the JSON of a pod.
type Change = fn(&mut Value);

/// The field path of the container of the pod `not-ready`.
const MAIN: Option<&str> = Some("spec.containers{main}");

/// A liveness probe of the default period on the container of the pod `not-ready`.
fn probed(pod: &mut Value) {
    pod["spec"]["containers"][0]["livenessProbe"] = json!({"exec": {"command": ["false"]}});
}

/// The state of the container of the pod `not-ready`.
fn in_state(pod: &mut Value, state: Value) {
    probed(pod);
    pod["status"]["containerStatuses"][0]["state"] = state;
}

/// The container of the pod `not-ready` is ready, and the pod has this reason of not ready.
fn ready_in(pod: &mut Value, reason: Value) {
    pod["status"]["containerStatuses"][0]["ready"] = true.into();
    condition(pod, "Ready")["reason"] = reason;
}

#[test]
fn each_fact_that_proves_neither_gets_the_evidence_of_the_pace() {
    let bed = Bed::new();
    let late = since_last(Duration::from_secs(3600));
    let of =
        |reason: &str, part: Option<&str>, message: &str| of_kubelet::<Pod>("not-ready", reason, LAST, part, message);
    let back_off = "Back-off restarting failed container main";
    let combined = format!("(combined from similar events): {back_off}");
    let readiness = of("Unhealthy", MAIN, "Readiness probe failed: ");
    let helper = Some("spec.initContainers{helper}");
    let sidecar = of_kubelet::<Pod>("sidecar", "Unhealthy", LAST, helper, "Liveness probe failed: ");
    let cases: [(&str, Event, Change, SystemTime, &str); 11] = [
        (
            "not-ready",
            liveness(),
            |pod| {
                in_state(pod, json!({"running": {"startedAt": "2026-10-13T18:10:00Z"}}));
                pod["status"]["containerStatuses"][0]["restartCount"] = 3.into();
            },
            moment("2026-10-13T18:15:00Z"),
            "the container main runs again after 3 restarts, for less than 10 min",
        ),
        (
            "not-ready",
            liveness(),
            |pod| in_state(pod, json!({"running": {}})),
            late,
            "the container main runs, and its status gives no time",
        ),
        (
            "not-ready",
            liveness(),
            |pod| in_state(pod, json!({"terminated": {"exitCode": 0}})),
            late,
            "the container main ended, and the kubelet starts it again",
        ),
        ("not-ready", liveness(), |pod| in_state(pod, json!({})), late, "the pod gives no state of the container main"),
        (
            "not-ready",
            readiness.clone(),
            |pod| ready_in(pod, "ContainersNotReady".into()),
            late,
            "the container main is ready, and the pod is not: ContainersNotReady",
        ),
        (
            "not-ready",
            readiness.clone(),
            |pod| ready_in(pod, Value::Null),
            late,
            "the container main is ready, and the pod is not",
        ),
        (
            "not-ready",
            readiness,
            |pod| {
                ready_in(pod, Value::Null);
                let conditions = pod["status"]["conditions"].as_array_mut().expect("conditions");
                conditions.retain(|condition| condition["type"] != "Ready");
            },
            late,
            "the pod has no condition Ready",
        ),
        ("not-ready", of("BackOff", MAIN, &combined), |_| {}, late, "the Event combines similar reports"),
        ("not-ready", of("BackOff", None, back_off), |_| {}, late, "the Event names no container"),
        (
            "not-ready",
            of("BackOff", Some("spec.containers{other}"), back_off),
            |_| {},
            late,
            "the pod gives no state of the container other",
        ),
        (
            "sidecar",
            sidecar,
            |pod| pod["spec"]["initContainers"][0]["livenessProbe"] = json!({"exec": {"command": ["false"]}}),
            late,
            "the container helper runs since before the last report",
        ),
    ];
    for (pod, event, change, time, why) in cases {
        let witness = bed.with_pod(pod, change, std::slice::from_ref(&event), time);
        let (state, said) = judged(&witness, &event);
        assert_eq!(state, Quiet(at(LAST).expect("a time")), "{said}");
        assert!(said.starts_with(why), "{said}");
    }
}
