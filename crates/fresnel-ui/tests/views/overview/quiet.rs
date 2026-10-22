//! The reason of each state on the row of a warning, and the quiet state. The user saw
//! "unknown" 35 minutes after the last report of a liveness probe that failed fewer times
//! in a row than its threshold, and asked what it means: the reason showed only at a hover.

use std::sync::Arc;
use std::time::Duration;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::ClusterRow;
use fresnel_core::index::IndexState;
use fresnel_core::store::StoreState;
use k8s_openapi::api::core::v1::Event;
use serde_json::json;

use super::warnings::{of, ready_again};
use super::{cluster, open, with};
use crate::data::{changed, changed_object, event_index, held, pod, pods, store};
use crate::fake::{Window, shows, until, view, window};
use crate::kinds::in_panel;
use crate::snapshots::ink;
use crate::workloads::scene::hover;

/// The pod of coredns of the scene. Its container runs since 07:00:00 in these tests.
const COREDNS: &str = "coredns-64fd4b4794-6lsjp";
/// The reason of the quiet state of its failed liveness probe.
const QUIET: &str = "the container coredns runs since before the last report; the kubelet repeats a problem that goes on at least \
     each 8 min";

/// A failed liveness probe of coredns, 4 times in a row up to `last`: fewer than its
/// threshold of 5, so the kubelet did not restart the container.
fn liveness(first: &str, last: &str) -> Arc<Event> {
    changed_object(&*of(COREDNS, "FailedCreatePodSandBox"), |event| {
        event["metadata"]["name"] = format!("{COREDNS}.liveness").into();
        (event["reason"], event["count"]) = ("Unhealthy".into(), 4.into());
        event["message"] = "Liveness probe failed: HTTP probe failed with statuscode: 503".into();
        event["involvedObject"]["fieldPath"] = "spec.containers{coredns}".into();
        (event["firstTimestamp"], event["lastTimestamp"]) = (first.into(), last.into());
    })
}

/// A cluster with these warnings in its store of the Events and in its Event index, whose
/// pod not-ready is ready again and whose coredns runs since 07:00:00.
fn with_warnings(warnings: &[Arc<Event>]) -> ClusterRow {
    let row = cluster("ams-prod", 1, "v1.33.5+k3s1");
    let mut data = row.data.clone().expect("data");
    let coredns = changed(&pod(COREDNS), |pod| {
        pod["status"]["containerStatuses"][0]["state"] = json!({"running": {"startedAt": "2027-01-15T07:00:00Z"}});
    });
    let names = [Some("not-ready"), Some(COREDNS)];
    let others = pods().into_iter().filter(|pod| !names.contains(&pod.metadata.name.as_deref()));
    data.pods = held(store(StoreState::Live, &others.chain([ready_again(), coredns]).collect::<Vec<_>>()));
    data.objects = with(data.objects, warnings);
    data.events = Some(event_index(store(StoreState::Live, warnings), IndexState::Following));
    ClusterRow { data: Some(data), ..row }
}

/// The three warnings of the question: the failed liveness probe of coredns with its last
/// report 35 min 24 s before 08:00:00, a failed readiness probe of a pod that is ready again,
/// and a warning of a pod that a rollout removed.
fn three_warnings() -> Window {
    let warnings = [
        liveness("2027-01-15T07:23:06Z", "2027-01-15T07:24:36Z"),
        of("not-ready", "Unhealthy"),
        of("web-64cb9bd5d4-c8kcm", "FailedToRetrieveImagePullSecret"),
    ];
    let mut window = window(view(vec![with_warnings(&warnings)]));
    open(&mut window, "Cluster overview");
    window
}

/// The place of the widget with this text.
fn rect(window: &Window, label: &str) -> egui::Rect {
    window.get_by_label(label).rect()
}

#[test]
fn each_row_has_the_reason_of_its_state_under_its_message_and_a_quiet_one_is_muted() {
    let mut window = three_warnings();
    assert!(shows(&window, "3 warnings in the last hour") && shows(&window, "1 quiet"));
    assert!(!shows(&window, "none ongoing") && !shows(&window, "1 unknown"), "a quiet one is no proof of an end");
    let chips = ["quiet 35 min", "resolved 07:59", "deleted"];
    let tops = chips.map(|chip| rect(&window, chip).top());
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "the quiet one before the ended ones: {tops:?}");
    let rows = [
        ("Liveness probe failed: HTTP probe failed with statuscode: 503", QUIET),
        ("Readiness probe failed:", "the container main is ready"),
        ("Unable to retrieve some image pull secrets (missing); attempting to pull the image may not succeed.", ""),
    ];
    for (message, reason) in rows.iter().filter(|(_, reason)| !reason.is_empty()) {
        let (message, reason) = (rect(&window, message), rect(&window, reason));
        assert!(reason.top() >= message.bottom() - 1.0 && (reason.left() - message.left()).abs() < 1.0, "{reason:?}");
    }
    assert!(shows(&window, "the Pod is not in its store"), "the reason of the deleted one");

    let frame = window.render().expect("a GPU adapter draws the frame");
    for label in ["quiet 35 min", QUIET] {
        let amber = ink(&frame, rect(&window, label)).0;
        assert!(amber < 20, "{label} is muted, not in the tone of a warning: {amber}");
    }
    hover(&mut window, Role::Label, "quiet 35 min");
    assert!(shows(&window, &format!("Quiet for 35 min, so most likely ended, but not proved: {QUIET}.")));
}

/// The state follows the clock of the worker: a warning whose last report was 15 min 30 s
/// ago is unknown, and quiet 30 s later with no new snapshot.
#[test]
fn a_warning_of_the_kubelet_turns_quiet_with_the_time_alone() {
    let warnings = [liveness("2027-01-15T07:43:00Z", "2027-01-15T07:44:30Z")];
    let mut window = window(view(vec![with_warnings(&warnings)]));
    open(&mut window, "Cluster overview");
    assert!(shows(&window, "1 unknown") && shows(&window, "unknown"), "the first minutes");
    assert!(shows(&window, "the container coredns runs since before the last report"), "the reason on the row");
    window.state().move_clock(Duration::from_secs(30));
    until(&mut window, "the quiet state", |window| shows(window, "quiet 16 min"));
    assert!(shows(&window, "1 quiet") && !shows(&window, "1 unknown") && shows(&window, QUIET));
}

/// The panel of the pod and the Now column of the Events give the same state and reason.
#[test]
fn the_panel_of_the_pod_and_the_events_page_show_the_quiet_state_and_its_reason() {
    let mut window = three_warnings();
    window.get_by_role_and_label(Role::Link, &format!("Pod {COREDNS}")).click();
    until(&mut window, "the panel of coredns", |window| in_panel(window, "quiet 35 min"));
    assert!(in_panel(&window, QUIET), "the reason beside the state");

    let mut window = three_warnings();
    window.get_by_role_and_label(Role::Link, "3 warnings in the last hour").click();
    let cell = format!("Quiet for 35 min, so most likely ended, but not proved: {QUIET}.");
    until(&mut window, "the Now column of the Events", |window| shows(window, &cell));
}
