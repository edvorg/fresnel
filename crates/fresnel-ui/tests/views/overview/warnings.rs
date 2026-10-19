//! The warnings of the Cluster overview with the state of each problem now. The user could
//! not tell if a warning of 31 minutes ago was still a problem, nor what shows at its end.

use std::sync::Arc;
use std::time::Duration;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::ClusterRow;
use fresnel_core::store::StoreState;
use k8s_openapi::api::core::v1::{Event, Pod};
use serde_json::json;

use super::{cluster, open, with};
use crate::data::{changed, changed_object, events, held, pod, pods, store};
use crate::fake::{Window, shows, until, view, window};
use crate::snapshots::ink;
use crate::workloads::scene::hover;

/// The Event of the scene about this object with this reason.
fn of(name: &str, reason: &str) -> Arc<Event> {
    let fits = |event: &Arc<Event>| {
        event.involved_object.name.as_deref() == Some(name) && event.reason.as_deref() == Some(reason)
    };
    events().into_iter().find(fits).unwrap_or_else(|| panic!("the scene has the {reason} of {name}"))
}

/// The top of the first widget with this name.
fn top(window: &Window, label: &str) -> f32 {
    window.get_by_label(label).rect().top()
}

/// The pod `not-ready` after its probe works again: its container is ready, and the pod is
/// ready since 07:59:50, after the last report of its failed probe at 07:59:45.
fn ready_again() -> Arc<Pod> {
    changed(&pod("not-ready"), |pod| {
        pod["status"]["containerStatuses"][0]["ready"] = true.into();
        pod["status"]["conditions"] = json!([{"type": "Ready", "status": "True",
            "lastTransitionTime": "2027-01-15T07:59:50Z"}]);
    })
}

/// A cluster with these warnings, whose store of the pods is in this state, and whose pod
/// `not-ready` is ready again.
fn with_warnings(warnings: &[Arc<Event>], pods_state: StoreState) -> ClusterRow {
    let row = cluster("ams-prod", 1, "v1.33.5+k3s1");
    let mut data = row.data.clone().expect("data");
    let others = pods().into_iter().filter(|pod| pod.metadata.name.as_deref() != Some("not-ready"));
    data.pods = held(store(pods_state, &others.chain([ready_again()]).collect::<Vec<_>>()));
    data.objects = with(data.objects, warnings);
    ClusterRow { data: Some(data), ..row }
}

/// A cluster with four warnings: a pod that still waits for a node (07:59:09), a failed
/// probe of a pod that is ready again (07:59:45), a CronJob warning that no rule judges
/// (07:59:28), and a pod of a ReplicaSet that a rollout removed (07:59:21).
fn four_warnings() -> ClusterRow {
    let warnings = [
        of("unschedulable", "FailedScheduling"),
        of("not-ready", "Unhealthy"),
        of("tick", "UnexpectedJob"),
        of("web-64cb9bd5d4-c8kcm", "FailedToRetrieveImagePullSecret"),
    ];
    with_warnings(&warnings, StoreState::Live)
}

#[test]
fn each_warning_shows_the_state_of_its_problem_now_and_the_ongoing_ones_come_first() {
    let mut window = window(view(vec![four_warnings()]));
    open(&mut window, "Cluster overview");
    let summary = ["4 warnings in the last hour", "1 ongoing", "1 unknown"];
    assert!(summary.iter().all(|text| shows(&window, text)), "the count, the ongoing ones and the unknown ones");
    let chips = ["ongoing", "unknown", "resolved 07:59", "deleted"];
    let tops: Vec<f32> = chips.iter().map(|chip| top(&window, chip)).collect();
    assert!(
        tops.windows(2).all(|pair| pair[0] < pair[1]),
        "the ongoing one, the unknown one, the ended ones: {tops:?}"
    );
    assert!(top(&window, "Pod unschedulable") < top(&window, "CronJob tick"), "a row of its chip");
    // The repeat count of a report, after its reason: the failed probe came 24 times.
    assert!(shows(&window, "×24") && shows(&window, "×3"));
    // The time has its name, so it does not read as the age of the problem or as its end.
    assert!(shows(&window, "last report 50 s ago") && shows(&window, "last report 15 s ago"));
    let middle = |label: &str| window.get_by_label(label).rect().center().y;
    assert!((middle("last report 15 s ago") - middle("resolved 07:59")).abs() < 2.0, "the time of its row");

    let hint = "A time is the last report of an Event: the cluster keeps an Event for about 1 hour after its last \
                report. The state comes from the live object now, not from the Event.";
    hover(&mut window, Role::Label, "deleted");
    assert!(shows(&window, "Deleted: the Pod is not in its store.") && !shows(&window, hint));
    hover(&mut window, Role::Label, "resolved 07:59");
    let resolved = "Resolved at 2027-01-15 07:59:50: the container main is ready.";
    assert!(shows(&window, resolved) && !shows(&window, hint), "the fact of the live object");
    // The mark after the heading tells that it has a hint.
    hover(&mut window, Role::Label, "WARNING EVENTS ?");
    assert!(shows(&window, hint), "what the time and the state tell");
}

/// A row of an ended problem must not look like an alarm with a grey word in front: only
/// the reason of an ongoing one has the amber tone of a warning, and the message of an ended
/// one is faint.
#[test]
fn the_row_of_an_ended_problem_is_muted_and_only_an_ongoing_one_has_the_tone_of_a_warning() {
    let mut window = window(view(vec![four_warnings()]));
    open(&mut window, "Cluster overview");
    let frame = window.render().expect("a GPU adapter draws the frame");
    let ink = |label: &str| ink(&frame, window.get_by_label(label).rect());
    let (ongoing, unknown) = (ink("FailedScheduling"), ink("UnexpectedJob"));
    let (resolved, deleted) = (ink("Unhealthy"), ink("FailedToRetrieveImagePullSecret"));
    assert!(ongoing.0 > 80, "the reason of the ongoing one is amber: {ongoing:?}");
    for (state, reason) in [("unknown", unknown), ("resolved", resolved), ("deleted", deleted)] {
        assert!(reason.0 < 20, "the reason of the {state} one is grey: {reason:?}");
    }
    let message = |event: &Arc<Event>| ink(event.message.as_deref().map(str::trim).expect("a message"));
    let ongoing = message(&of("unschedulable", "FailedScheduling"));
    let ended = [of("not-ready", "Unhealthy"), of("web-64cb9bd5d4-c8kcm", "FailedToRetrieveImagePullSecret")];
    assert!(ongoing.1 > 0xc0, "the message of the ongoing one has the tone of the text: {ongoing:?}");
    for ended in ended.iter().map(message) {
        assert!(ended.1 < 0x90, "the message of an ended one is faint: {ended:?}");
    }
}

/// The count of the ongoing ones is of each warning of the last hour: the scene has 9, and
/// a block shows 8 rows.
#[test]
fn the_count_of_the_ongoing_warnings_is_of_each_warning_and_not_only_of_the_shown_rows() {
    let mut window = window(view(vec![cluster("ams-prod", 1, "v1.33.5+k3s1")]));
    open(&mut window, "Cluster overview");
    assert!(shows(&window, "31 warnings in the last hour") && shows(&window, "9 ongoing"));
    assert!(shows(&window, "6 unknown"), "a state that the app cannot tell is no end");
    let chips = window.query_all_by_label("ongoing").filter(|node| node.accesskit_node().role() == Role::Label);
    assert_eq!(chips.count(), 8, "each shown row is ongoing");
}

/// "None ongoing" tells that no problem stays, so it shows only when the app knows the state
/// of each warning: a store of the pods that is not live, or a warning with no rule, leaves
/// a state unknown.
#[test]
fn none_ongoing_shows_only_when_the_state_of_each_warning_is_known() {
    let probe = [of("not-ready", "Unhealthy")];
    let mut known = window(view(vec![with_warnings(&probe, StoreState::Live)]));
    open(&mut known, "Cluster overview");
    assert!(shows(&known, "1 warning in the last hour") && shows(&known, "none ongoing"), "the pod is ready again");

    let mut old = window(view(vec![with_warnings(&probe, StoreState::Stopped)]));
    open(&mut old, "Cluster overview");
    assert!(shows(&old, "1 unknown") && !shows(&old, "none ongoing"), "the pods are not live");

    let two = [of("not-ready", "Unhealthy"), of("tick", "UnexpectedJob")];
    let mut some = window(view(vec![with_warnings(&two, StoreState::Live)]));
    open(&mut some, "Cluster overview");
    assert!(shows(&some, "2 warnings in the last hour") && shows(&some, "1 unknown"));
    assert!(!shows(&some, "none ongoing"), "one is resolved, and the state of the other is not known");
}

/// The cluster keeps an Event after the hour: the count of the last hour follows the clock
/// with no new snapshot, so an old warning does not count as one of the last hour.
#[test]
fn a_warning_leaves_the_last_hour_with_the_time_alone() {
    let early = changed_object(&*of("unschedulable", "FailedScheduling"), |event| {
        event["lastTimestamp"] = "2027-01-15T07:00:05Z".into();
        event["eventTime"] = serde_json::Value::Null;
        event["series"] = serde_json::Value::Null;
    });
    let mut window = window(view(vec![with_warnings(&[early], StoreState::Live)]));
    open(&mut window, "Cluster overview");
    assert!(shows(&window, "1 warning in the last hour") && shows(&window, "1 ongoing"), "07:00:05 at 08:00:00");
    // The hour starts at 07:00:10 one second later, at the next step of 10 s.
    window.state().move_clock(Duration::from_secs(1));
    until(&mut window, "a count of the new hour", |window| shows(window, ", 0 in the last hour"));
    assert!(window.query_by_label("Pod unschedulable").is_none(), "no row of the old warning");
    // The link has the count of the rows of its table, which keeps the old warning.
    window.get_by_role_and_label(Role::Link, "1 warning").click();
    until(&mut window, "the warnings", |window| shows(window, "1 Event"));
}

/// A custom resource can have the name of a kind of the app: a Service of Knative is not
/// the Service of its name, so its row names it with no link, and its state is unknown.
#[test]
fn a_warning_of_a_custom_resource_with_the_name_of_a_kind_has_no_link() {
    let knative = changed_object(&*of("unschedulable", "FailedScheduling"), |event| {
        let object = &mut event["involvedObject"];
        (object["kind"], object["apiVersion"], object["name"]) =
            ("Service".into(), "serving.knative.dev/v1".into(), "web".into());
        event["reason"] = "RevisionFailed".into();
    });
    let mut window = window(view(vec![with_warnings(&[knative], StoreState::Live)]));
    open(&mut window, "Cluster overview");
    assert!(shows(&window, "Service web") && shows(&window, "unknown"), "the row of the warning");
    assert!(window.query_by_role_and_label(Role::Link, "Service web").is_none(), "no link to the Service web");
}
