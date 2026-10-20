//! The pace of the reports: a warning of the kubelet whose live object proves neither is
//! quiet after a time with no report of its object, and stays unknown before it, for an
//! other source, with a newer report of its object, or with Events that the app may miss.

use std::time::{Duration, SystemTime};

use super::*;
use crate::fixture::condition;
use crate::index::{Index, IndexState, ObjectEvents};
use crate::published::{Notice, Publisher};

/// The last report of each warning of these tests.
const LAST: &str = "2026-10-13T17:58:50Z";
/// The reason of a failed liveness probe of a container that runs since 17:58:27.
const RUNS: &str = "the container main runs since before the last report";

/// A warning of the kubelet about an object of the scene, with its last report at `last`.
fn of_kubelet<K: Describe>(name: &str, reason: &str, last: &str, part: Option<&str>, message: &str) -> Event {
    let mut event = warning::<K>(name, reason, last, part);
    event.message = Some(message.to_owned());
    event.source = Some(serde_json::from_value(json!({"component": "kubelet", "host": NODE})).expect("a source"));
    event
}

/// A failed liveness probe of the pod `not-ready`.
fn liveness() -> Event {
    of_kubelet::<Pod>("not-ready", "Unhealthy", LAST, Some("spec.containers{main}"), "Liveness probe failed: 500")
}

/// The moment at `silent` after the last report.
fn since_last(silent: Duration) -> SystemTime {
    moment(LAST) + silent
}

impl Bed {
    /// The Event index of these Events on a store in this state, which dropped `dropped`.
    fn reports(&self, events: &[Event], state: StoreState, dropped: u64) -> (Held<Index<ObjectEvents>>, StoreStatus) {
        let since = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let items = events.iter().cloned().map(Arc::new).collect();
        let snapshot = Snapshot { generation: 1, state, since, last_contact: None, listed: true, dropped, items };
        let index = Index::<ObjectEvents>::of_snapshot(&snapshot, IndexState::Following);
        (Publisher::new(index, self.disposal.bin(), Notice::new()).read(), snapshot.status(Kind::Event))
    }

    /// The scene with a liveness probe of this period on the container of `not-ready`, the
    /// index of these Events in a live store, at `time`.
    fn probed(&self, period: Option<i64>, events: &[Event], time: SystemTime) -> Witness {
        let witness = self.with::<Pod>("not-ready", |pod| {
            let probe = json!({"exec": {"command": ["false"]}, "failureThreshold": 1000, "periodSeconds": period});
            pod["spec"]["containers"][0]["livenessProbe"] = probe;
        });
        let (index, store) = self.reports(events, StoreState::Live, 0);
        Witness { at: time, ..witness }.with_reports(index, &store)
    }
}

/// The user saw "unknown" 35 minutes after the last report of a liveness probe that failed
/// fewer times than its threshold: no field of the pod tells if the probe works now.
#[test]
fn an_undecided_warning_of_the_kubelet_is_quiet_after_twice_its_pace_with_no_report() {
    let bed = Bed::new();
    let event = liveness();
    let before = bed.probed(Some(10), std::slice::from_ref(&event), since_last(QUIET_AFTER - Duration::from_secs(1)));
    assert_eq!(judged(&before, &event), state(Unknown, RUNS), "the first minutes");
    let quiet = bed.probed(Some(10), std::slice::from_ref(&event), since_last(QUIET_AFTER));
    let why = format!("the kubelet reports a problem that goes on again at least each 8 min, and {RUNS}");
    assert_eq!(judged(&quiet, &event), (Quiet(at(LAST).expect("a time")), why));
    assert_eq!((REPORT_PACE, QUIET_AFTER), (Duration::from_secs(480), Duration::from_secs(960)));
    let default_period = bed.probed(None, std::slice::from_ref(&event), since_last(QUIET_AFTER));
    assert_eq!(judged(&default_period, &event).0, Quiet(at(LAST).expect("a time")), "a period of 10 s");

    // The newer recorder writes the reporting component too, and may write no source.
    let mut reported = event.clone();
    (reported.source, reported.reporting_component) = (None, Some("kubelet".to_owned()));
    assert_eq!(judged(&quiet, &reported).0, Quiet(at(LAST).expect("a time")));
}

/// The pace of the kubelet is measured; the scheduler writes a series each 30 min, and the
/// pace of a controller is not known.
#[test]
fn a_warning_of_an_other_source_stays_unknown() {
    let bed = Bed::new();
    let late = since_last(Duration::from_secs(3600));
    let mut scheduler = liveness();
    scheduler.source = Some(serde_json::from_value(json!({"component": "default-scheduler"})).expect("a source"));
    let mut none = liveness();
    none.source = None;
    for event in [scheduler, none] {
        let witness = bed.probed(Some(10), std::slice::from_ref(&event), late);
        assert_eq!(judged(&witness, &event), state(Unknown, RUNS), "{:?}", event.source);
    }
    let placed = bed.with::<Pod>("unschedulable", |pod| {
        let scheduled = condition(pod, "PodScheduled");
        (scheduled["status"], scheduled["lastTransitionTime"]) = ("True".into(), EARLY.into());
    });
    let mut waits = of_kubelet::<Pod>("unschedulable", "FailedScheduling", LAST, None, "0/1 nodes are available");
    waits.source = Some(serde_json::from_value(json!({"component": "default-scheduler"})).expect("a source"));
    let (index, store) = bed.reports(std::slice::from_ref(&waits), StoreState::Live, 0);
    let placed = Witness { at: late, ..placed }.with_reports(index, &store);
    assert_eq!(judged(&placed, &waits), state(Unknown, "the pod has a node since before the last report"));
}

/// A rule that proves the problem goes on or ended wins, and a state that is no fact of the
/// live object (a pod that failed, a store that is not live, no rule) gets no evidence.
#[test]
fn a_proof_and_a_state_that_is_no_fact_of_the_object_win_over_quiet() {
    let bed = Bed::new();
    let late = since_last(Duration::from_secs(3600));
    let back_off = of_kubelet::<Pod>("crash", "BackOff", LAST, Some("spec.containers{crash}"), "Back-off restarting");
    let (index, store) = bed.reports(std::slice::from_ref(&back_off), StoreState::Live, 0);
    let scene = bed.scene(&fixture::all(), &fixture::all());
    let witness = Witness { at: late, ..scene }.with_reports(index.clone(), &store);
    assert_eq!(judged(&witness, &back_off).0, Ongoing, "the container waits");
    let failed = bed.with::<Pod>("crash", |pod| pod["status"]["phase"] = "Failed".into());
    let failed = Witness { at: late, ..failed }.with_reports(index.clone(), &store);
    assert_eq!(judged(&failed, &back_off), state(Unknown, "the pod failed, so no container of it runs"));
    let paused = StoreState::Stopped;
    let old = Witness::new([bed.source(Kind::Pod, paused, &fixture::all::<Pod>())], late).with_reports(index, &store);
    assert_eq!(judged(&old, &back_off), state(Unknown, "the store of the Pods is not live: stopped"));
    let no_rule = of_kubelet::<Node>(NODE, "InvalidDiskCapacity", LAST, None, "invalid capacity 0 on image filesystem");
    let (index, store) = bed.reports(std::slice::from_ref(&no_rule), StoreState::Live, 0);
    let witness = Witness { at: late, ..bed.scene(&fixture::all(), &fixture::all()) }.with_reports(index, &store);
    assert_eq!(judged(&witness, &no_rule).0, Unknown, "a reason that the kubelet may report one time");
}

/// The warnings of one object share its turns: on k3s the failed liveness probe of a pod
/// took each turn, and its failed readiness probe got no report for 15 min while it failed.
#[test]
fn a_newer_report_of_the_object_or_events_that_the_app_can_miss_keep_it_unknown() {
    let bed = Bed::new();
    let late = since_last(Duration::from_secs(3600));
    let event = liveness();
    let newer = "2026-10-13T18:20:00Z";
    let mut readiness =
        of_kubelet::<Pod>("not-ready", "Unhealthy", newer, Some("spec.containers{main}"), "Readiness probe failed: ");
    readiness.metadata.name = Some("not-ready.2".to_owned());
    let witness = bed.probed(Some(10), &[event.clone(), readiness.clone()], late);
    assert_eq!(judged(&witness, &event), state(Unknown, RUNS), "the readiness probe can have taken its turn");
    let mut again = event.clone();
    again.last_timestamp = readiness.last_timestamp.clone();
    let witness = bed.probed(Some(10), std::slice::from_ref(&again), late);
    assert_eq!(judged(&witness, &event), state(Unknown, RUNS), "the index has a newer report of the same Event");

    // A newer Event that does not share the turns of the warnings of the kubelet.
    let mut normal = readiness.clone();
    (normal.type_, normal.metadata.name) = (Some("Normal".to_owned()), Some("not-ready.3".to_owned()));
    let mut scheduler = readiness.clone();
    scheduler.metadata.name = Some("not-ready.4".to_owned());
    scheduler.source = Some(serde_json::from_value(json!({"component": "default-scheduler"})).expect("a source"));
    let witness = bed.probed(Some(10), &[event.clone(), normal, scheduler], late);
    assert_eq!(judged(&witness, &event).0, Quiet(at(LAST).expect("a time")));

    // An index that does not follow a live store, a store that dropped Events, or no index.
    let probed = bed.probed(Some(10), &[], late);
    for (state, dropped) in [(StoreState::Stopped, 0), (StoreState::Live, 3)] {
        let (index, store) = bed.reports(std::slice::from_ref(&event), state.clone(), dropped);
        let witness = Witness { reports: None, ..probed.clone() }.with_reports(index, &store);
        assert_eq!(judged(&witness, &event), super::state(Unknown, RUNS), "{state}, {dropped} dropped");
    }
    assert_eq!(judged(&Witness { reports: None, ..probed }, &event), state(Unknown, RUNS));
}

/// The kubelet tries the report of a failed probe at each of its runs: a probe that runs
/// seldom, or one that the app cannot find, gets no evidence of the pace.
#[test]
fn a_probe_that_runs_seldom_or_that_the_spec_does_not_give_stays_unknown() {
    let bed = Bed::new();
    let late = since_last(Duration::from_secs(3600));
    let event = liveness();
    let slow = bed.probed(Some(300), std::slice::from_ref(&event), late);
    assert_eq!(judged(&slow, &event), state(Unknown, RUNS), "a period of 5 min");
    let edge = bed.probed(Some(125), std::slice::from_ref(&event), late);
    assert_eq!(judged(&edge, &event).0, Quiet(at(LAST).expect("a time")), "a period of the longest retry");
    let (index, store) = bed.reports(std::slice::from_ref(&event), StoreState::Live, 0);
    let no_probe = Witness { at: late, ..bed.scene(&fixture::all(), &fixture::all()) }.with_reports(index, &store);
    assert_eq!(judged(&no_probe, &event), state(Unknown, RUNS), "the spec has no liveness probe");
    let mut combined = event.clone();
    combined.message = Some("(combined from similar events): Liveness probe failed: 500".to_owned());
    let combined_why = "the Event combines similar reports, which can be of an other container";
    let witness = bed.probed(Some(10), std::slice::from_ref(&combined), late);
    assert_eq!(judged(&witness, &combined), state(Unknown, combined_why));
}

/// A sync of the kubelet retries a sandbox and a mount, and its eviction manager checks
/// the node each 10 s: a state "since before the last report" of their objects is quiet.
#[test]
fn a_sandbox_a_mount_and_a_shortage_of_the_node_are_quiet_too() {
    let bed = Bed::new();
    let late = since_last(Duration::from_secs(3600));
    let scene = || Witness { at: late, ..bed.scene(&fixture::all(), &fixture::all()) };
    for reason in ["FailedCreatePodSandBox", "FailedMount"] {
        let event = of_kubelet::<Pod>("crash", reason, LAST, None, "failed");
        let (index, store) = bed.reports(std::slice::from_ref(&event), StoreState::Live, 0);
        let (state, why) = judged(&scene().with_reports(index, &store), &event);
        assert_eq!(state, Quiet(at(LAST).expect("a time")), "{reason}");
        assert!(why.ends_with("since before the last report"), "{why}");
    }
    let shortage = of_kubelet::<Node>(NODE, "EvictionThresholdMet", LAST, None, "Attempting to reclaim memory");
    let calm = bed.with::<Node>(NODE, |node| {
        let disk = condition(node, "DiskPressure");
        (disk["status"], disk["lastTransitionTime"]) = ("False".into(), EARLY.into());
    });
    let (index, store) = bed.reports(std::slice::from_ref(&shortage), StoreState::Live, 0);
    let calm = Witness { at: late, ..calm }.with_reports(index, &store);
    assert_eq!(judged(&calm, &shortage).0, Quiet(at(LAST).expect("a time")), "the kubelet names its node");
}
