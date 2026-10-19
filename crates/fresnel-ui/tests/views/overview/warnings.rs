//! The warnings of the Cluster overview with the state of each problem now. The user could
//! not tell if a warning of 31 minutes ago was still a problem, nor what shows at its end.

use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::ClusterRow;
use fresnel_core::store::StoreState;
use k8s_openapi::api::core::v1::{Event, Pod};
use serde_json::json;

use super::{cluster, open, with};
use crate::data::{changed, events, held, pod, pods, store};
use crate::fake::{Window, shows, view, window};
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

/// The pod `not-ready` after its probe works again: ready since 07:59:50, after the last
/// report of its failed probe at 07:59:45.
fn ready_again() -> Arc<Pod> {
    changed(&pod("not-ready"), |pod| {
        pod["status"]["conditions"] = json!([{"type": "Ready", "status": "True",
            "lastTransitionTime": "2027-01-15T07:59:50Z"}]);
    })
}

/// A cluster with four warnings: a pod that still waits for a node (07:59:09), a failed
/// probe of a pod that is ready again (07:59:45), a CronJob warning that no rule judges
/// (07:59:28), and a pod of a ReplicaSet that a rollout removed (07:59:21).
fn four_warnings() -> ClusterRow {
    let row = cluster("ams-prod", 1, "v1.33.5+k3s1");
    let mut data = row.data.clone().expect("data");
    let others = pods().into_iter().filter(|pod| pod.metadata.name.as_deref() != Some("not-ready"));
    data.pods = held(store(StoreState::Live, &others.chain([ready_again()]).collect::<Vec<_>>()));
    let warnings = [
        of("unschedulable", "FailedScheduling"),
        of("not-ready", "Unhealthy"),
        of("tick", "UnexpectedJob"),
        of("web-64cb9bd5d4-c8kcm", "FailedToRetrieveImagePullSecret"),
    ];
    data.objects = with(data.objects, &warnings);
    ClusterRow { data: Some(data), ..row }
}

#[test]
fn each_warning_shows_the_state_of_its_problem_now_and_the_ongoing_ones_come_first() {
    let mut window = window(view(vec![four_warnings()]));
    open(&mut window, "Cluster overview");
    let summary = ["4 warnings in the last hour", "1 ongoing", "1 unknown"];
    assert!(summary.iter().all(|text| shows(&window, text)), "the count, the ongoing ones and the unknown ones");
    let chips = ["ongoing", "resolved 07:59", "unknown", "gone"];
    let tops: Vec<f32> = chips.iter().map(|chip| top(&window, chip)).collect();
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "the ongoing one first, then the newest: {tops:?}");
    assert!(top(&window, "Pod unschedulable") < top(&window, "Pod not-ready"), "a row of its chip");
    // The repeat count of a report, after its reason: the failed probe came 24 times.
    assert!(shows(&window, "×24") && shows(&window, "×3"));

    hover(&mut window, Role::Label, "resolved 07:59");
    assert!(shows(&window, "Resolved at 2027-01-15 07:59:50: the pod is ready."), "the fact of the live object");
    hover(&mut window, Role::Label, "gone");
    assert!(shows(&window, "Gone: the Pod is not in its store."));
    hover(&mut window, Role::Label, "WARNING EVENTS");
    let hint = "A time is the last report of an Event: the cluster keeps an Event for about 1 hour after its last \
                report. The state comes from the live object now, not from the Event.";
    assert!(shows(&window, hint), "what the time and the state tell");
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
