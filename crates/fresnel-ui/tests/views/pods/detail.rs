//! The detail panel of a pod: its metadata, its Events and what the panel knows of them.

use std::sync::Arc;

use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::{ClusterData, ClusterRow};
use fresnel_core::index::IndexState;
use fresnel_core::store::{Snapshot, StoreState};
use k8s_openapi::api::core::v1::Pod;

use crate::data::{changed, cluster, data, empty_store, event_index, events, live, pod, pods, renamed, store};
use crate::fake::{SECOND, Window, WithData as _, absent, connected, live_status, now, shows, until, view, window};
use crate::pods::scenes::no_answer;

fn open(window: &mut Window, name: &str) {
    window.get_by_label(name).click();
    window.run();
}

/// True when the detail panel shows this text.
fn in_panel(window: &Window, text: &str) -> bool {
    let panel = window.get_by_label("METADATA").rect().left();
    window.query_all_by_label(text).any(|node| node.rect().left() >= panel)
}

/// The pods of the scene with one of them replaced or removed.
fn pods_with(name: &str, new: Option<Arc<Pod>>) -> Vec<Arc<Pod>> {
    let others = pods().into_iter().filter(|pod| pod.metadata.name.as_deref() != Some(name));
    others.chain(new).collect()
}

#[test]
fn a_click_on_a_row_opens_the_panel_with_the_metadata_and_the_events_of_that_pod() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    assert!(!shows(&window, "Close the details"), "no row is selected");
    open(&mut window, "db-0");
    let uid = pod("db-0").metadata.uid.clone().expect("a UID");
    let texts = [
        "Pod",
        "METADATA",
        "Namespace",
        "Cluster",
        "UID",
        uid.as_str(),
        "Created",
        "2027-01-15 07:59:09 (51 s ago)",
        "Node",
        "Controlled by",
        "StatefulSet db",
        "Labels",
        "app=db",
        "apps.kubernetes.io/pod-index=0",
        "Annotations",
        "EVENTS",
        "Scheduled",
        "Pulled",
        "Created",
        "Started",
    ];
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
    assert!(window.query_all_by_label("db-0").count() >= 2, "the row and the heading of the panel");
    assert!(window.query_all_by_label_contains("Successfully assigned scene/db-0 to ").next().is_some());
    assert!(!shows(&window, "No Events."));

    // The keys move the cursor, and the panel follows it.
    window.key_press(egui::Key::ArrowDown);
    window.run();
    assert!(shows(&window, "apps.kubernetes.io/pod-index=1"), "the panel shows db-1");
    window.get_by_label("Close the details").click();
    window.run();
    assert!(!shows(&window, "METADATA"));
    window.key_press(egui::Key::ArrowDown);
    window.run();
    assert!(!shows(&window, "METADATA"), "the close removed the selection too");
}

/// Plan section 11.2, and defect A1b of the prototype: the panel builds its data again
/// for each snapshot, and a deleted object shows as deleted.
#[test]
fn the_panel_shows_each_change_of_its_pod_and_a_pod_that_went_away_as_deleted() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    open(&mut window, "db-0");
    assert!(in_panel(&window, "Running") && !shows(&window, "Evicted"));

    // The kubelet evicts the pod: the same object in a new version.
    let evicted = changed(&pod("db-0"), |value| value["status"]["reason"] = "Evicted".into());
    window.state_mut().view = view(vec![live("ams-prod", &pods_with("db-0", Some(evicted)))]);
    window.step();
    assert!(in_panel(&window, "Evicted"), "the panel reads the snapshot of each frame");
    until(&mut window, "the row of the new version", |window| {
        crate::pods::column(window, "Status").contains(&"Evicted".to_owned())
    });

    window.state_mut().view = view(vec![live("ams-prod", &pods_with("db-0", None))]);
    window.step();
    assert!(shows(&window, "This pod is deleted. The panel shows its last state."));
    assert!(shows(&window, "apps.kubernetes.io/pod-index=0"), "the last state stays");
    until(&mut window, "the rows with no db-0", |window| shows(window, "24 pods"));
    assert_eq!(window.query_all_by_label("db-0").count(), 1, "the heading of the panel only");

    // A new pod of the StatefulSet has the name. It is an other object.
    let next = renamed(&pod("db-0"), "db-0", "the-uid-of-the-next-pod");
    window.state_mut().view = view(vec![live("ams-prod", &pods_with("db-0", Some(next)))]);
    until(&mut window, "the row of the new pod", |window| shows(window, "25 pods"));
    assert!(shows(&window, "This pod is deleted. The panel shows its last state."));
    assert!(!shows(&window, "the-uid-of-the-next-pod"));
    window.get_by_label("Close the details").click();
    window.run();
    assert!(!shows(&window, "This pod is deleted. The panel shows its last state."));
}

/// A store with no complete list does not know its pods. That is not a deleted pod.
#[test]
fn a_store_that_lost_its_list_does_not_make_the_pod_of_the_panel_a_deleted_pod() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    open(&mut window, "crash");
    let listing = empty_store(StoreState::Listing { loaded: 0 }, now() - SECOND);
    window.state_mut().view = view(vec![cluster("ams-prod", listing)]);
    window.step();
    let text = "The pods of ams-prod are not loaded now. The panel shows the last state of this pod.";
    assert!(shows(&window, text) && !shows(&window, "This pod is deleted. The panel shows its last state."));

    window.state_mut().view = view(vec![live("ams-prod", &pods())]);
    until(&mut window, "the rows of the new list", |window| shows(window, "25 pods"));
    assert!(shows(&window, "METADATA") && !shows(&window, text), "the pod is in the new list");
}

/// What the Event section shows for each answer of the Event index. "No Events" shows
/// only for an index that knows it.
#[test]
fn the_event_section_tells_what_its_events_are_good_for() {
    let events_in = |state: StoreState, listed: bool| Snapshot { listed, ..store(state, &events()) };
    let pause = StoreState::Backoff { failures: 2, reason: no_answer(), until: now() + SECOND };
    let with_events = |events| ClusterData { events, ..data(store(StoreState::Live, &pods())) };
    let on = |events: Snapshot<_>, state| Some(event_index(events, state));
    let failed = IndexState::Failed { reason: "the task of the index panicked: a defect".into() };
    let cases: [(Option<_>, &str, bool); 6] = [
        (on(events_in(StoreState::Live, true), IndexState::Following), "", true),
        (
            on(events_in(pause.clone(), true), IndexState::Following),
            "The Events can be old: their store pause after 2 failures: the list of the pods got no answer in 15s.",
            true,
        ),
        (
            on(events_in(StoreState::Listing { loaded: 40 }, false), IndexState::Following),
            "The Events are not loaded: their store list in progress, 40 objects.",
            true,
        ),
        (
            on(events_in(StoreState::Live, true), failed),
            "The Events can be old: the Event index failed (the task of the index panicked: a defect). It starts again.",
            true,
        ),
        (
            on(Snapshot { listed: false, ..store(StoreState::Listing { loaded: 0 }, &[]) }, IndexState::Following),
            "The Events are not loaded: their store list in progress, 0 objects.",
            false,
        ),
        (None, "The settings give the Events of this cluster no store.", false),
    ];
    for (index, note, has_events) in cases {
        let row: ClusterRow = connected("ams-prod", live_status()).with(Some(with_events(index)));
        let mut window = window(view(vec![row]));
        open(&mut window, "crash");
        assert!(note.is_empty() || shows(&window, note), "{note}");
        assert_eq!(shows(&window, "BackOff"), has_events, "{note}: the Events of the pod");
        assert!(!shows(&window, "No Events."), "{note}: the index does not know that the pod has none");
    }

    // A pod with no Event in a live index: only this answer is "none".
    let lonely = renamed(&pod("crash"), "lonely", "a-uid-with-no-event");
    let mut window = window(view(vec![live("ams-prod", &[lonely])]));
    open(&mut window, "lonely");
    assert!(shows(&window, "No Events.") && !shows(&window, "BackOff"));
}
