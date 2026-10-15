//! The overviews at their edges: a stream of snapshots, and texts of a cluster with no
//! limit of their length.

use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::ClusterRow;
use fresnel_core::cluster::{ServerVersion, VersionState};
use fresnel_core::store::StoreState;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Event;

use super::{cluster, open, with};
use crate::data::{empty_store, events, held, pods, store};
use crate::fake::{SECOND, error, now, open_page, shows, until, view, window};

/// The cluster of the scene with its first `count` pods.
fn with_pods(count: usize) -> ClusterRow {
    let row = cluster("ams-prod", 1, "v1.33.5+k3s1");
    let mut data = row.data.clone().expect("data");
    data.pods = held(store(StoreState::Live, pods().get(..count).expect("the pods")));
    ClusterRow { data: Some(data), ..row }
}

/// A busy cluster gives new snapshots each few frames, and each one asks for new counts
/// that come some milliseconds later. The page tells that it counts only after a wait, as
/// the table does: a note in each frame after a change would flicker.
#[test]
fn a_stream_of_snapshots_gives_new_counts_with_no_note_in_the_frame_of_each_change() {
    let mut window = window(view(vec![with_pods(25)]));
    open(&mut window, "Cluster overview");
    let mut noted = 0;
    for round in 0..20 {
        let count = 24 + round % 2;
        window.state_mut().view = view(vec![with_pods(count)]);
        window.step();
        noted += usize::from(window.query_by_label("counting…").is_some());
        until(&mut window, "the new counts", |window| shows(window, &format!("{count} pods")));
    }
    assert_eq!(noted, 0, "frames of a change with the note");
}

/// A warning Event of this name about the object of this kind and name, with this reason.
fn warning(event_name: &str, (kind, name): (&str, &str), reason: &str) -> Arc<Event> {
    let first = events().into_iter().find(|event| event.type_.as_deref() == Some("Warning"));
    let mut event = Event::clone(&first.expect("a warning of the scene"));
    event.metadata.name = Some(event_name.to_owned());
    (event.involved_object.kind, event.involved_object.name) = (Some(kind.to_owned()), Some(name.to_owned()));
    event.reason = Some(reason.to_owned());
    Arc::new(event)
}

/// Bad data of a cluster: the API gives a version, the reason and the object of an Event and
/// the message of an error no limit, and the layout of a text of 1 MB takes longer than a
/// frame. Each line of the overviews shows the start of such a text.
#[test]
fn a_text_of_the_cluster_with_no_limit_shows_its_start_in_each_line_of_the_overviews() {
    let long = "x".repeat(100_000);
    let row = cluster("ams-prod", 1, "v1.33.5+k3s1");
    let mut data = row.data.clone().expect("data");
    let failed = VersionState::Failed { error: error(&long), at: now() - 10 * SECOND };
    data.version = held(ServerVersion { version: Some(long.clone().into()), state: failed });
    let events = [warning("a", ("Pod", &long), &long), warning("b", (&long, "web"), "BackOff")];
    data.objects = with(data.objects, &events);
    let backoff = StoreState::Backoff { failures: 3, reason: error(&long), until: now() + 5 * SECOND };
    data.objects = data.objects.with(held(empty_store::<Deployment>(backoff, now() - 20 * SECOND)));
    let mut window = window(view(vec![ClusterRow { data: Some(data), ..row }]));
    let cut = format!("{}…", "x".repeat(80));
    let too_long = "x".repeat(161);

    open(&mut window, "Cluster overview");
    let link = window.query_all_by_label(&format!("Pod {cut}")).any(|node| node.accesskit_node().role() == Role::Link);
    assert!(link, "the object of the Event, as a link");
    assert!(shows(&window, &cut), "the reason of the Event");
    assert!(shows(&window, &format!("{cut} web")), "an object of a kind that the app does not know");
    let version = format!("{cut}, old: the read of ");
    assert!(window.query_all_by_label_contains(&version).next().is_some(), "the version, with the start of the error");
    assert_eq!(window.query_all_by_label_contains(&too_long).count(), 0, "a line with all of a text");

    open_page(&mut window, "Workloads overview");
    until(&mut window, "the mark of the Deployments", |window| {
        window.query_all_by_label_contains("not loaded: the store").next().is_some()
    });
    assert_eq!(window.query_all_by_label_contains(&too_long).count(), 0, "a mark with all of an error");
}
