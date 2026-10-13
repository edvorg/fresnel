//! The weaknesses of Headlamp of plan section 6 that apply to the Pods view. Each test
//! has the number of its line in that section.

use egui_kittest::kittest::Queryable as _;
use fresnel_core::cluster::LinkState;
use fresnel_core::net::{ChannelState, ChannelStatus};
use fresnel_core::store::{StoreState, Wait};

use crate::data::{cluster, data, empty_store, in_state, live, pods, store};
use crate::fake::{SECOND, WithData as _, channel, connected, error, now, row, shows, status, until, view, window};
use crate::pods::scenes::{forbidden, no_answer, pods_of};
use crate::pods::{column, marks};

const MARK: &str = "old since 07:59:12";

/// Weakness 1: data from the first selected cluster only.
#[test]
fn w1_the_table_and_its_count_have_the_pods_of_each_selected_cluster() {
    // The two clusters run the same manifests: each name and each UID is in the two.
    let mut window = window(view(vec![live("ams-prod", &pods()), live("fra-stage", &pods())]));
    assert!(shows(&window, "50 pods"), "the count is of the two clusters");
    let rows: Vec<(String, String)> = column(&window, "Name").into_iter().zip(column(&window, "Cluster")).collect();
    let expected = [("agent-8djvd", "ams-prod"), ("agent-8djvd", "fra-stage"), ("broken-6bf768c954-gjvsw", "ams-prod")];
    assert_eq!(rows[..3], expected.map(|(name, cluster)| (name.to_owned(), cluster.to_owned())));

    // A row is an object of one cluster. A click on the second `crash` opens that pod.
    let second = window.query_all_by_label("crash").nth(1).expect("the pod of the second cluster");
    second.click();
    window.run();
    assert!(shows(&window, "Close the details") && window.query_all_by_label("fra-stage").count() >= 3);
    assert!(window.query_all_by_label_contains(" selected").next().is_none(), "one pod is selected, not two");

    // A cluster that leaves the selection takes its rows with it, and its panel.
    window.state_mut().view = view(vec![live("ams-prod", &pods())]);
    until(&mut window, "the rows of one cluster", |window| shows(window, "25 pods"));
    assert!(!shows(&window, "Close the details") && !column(&window, "Cluster").contains(&"fra-stage".to_owned()));
}

/// Weakness 3: only the first error is kept.
#[test]
fn w3_each_cluster_that_fails_shows_its_own_error_and_the_other_clusters_keep_their_rows() {
    let pause = StoreState::Backoff { failures: 3, reason: no_answer(), until: now() + 2 * SECOND };
    let clusters = vec![
        cluster("nyc-prod", empty_store(StoreState::Forbidden { reason: forbidden() }, now() - 20 * SECOND)),
        cluster("sgp-prod", store(pause, &pods_of("sgp"))),
        row("lab", LinkState::Failed(error("the cluster entry `lab` has no certificate authority"))),
        live("ams-prod", &pods_of("ams")),
    ];
    let window = window(view(clusters));
    let errors = [
        "Pods: forbidden: pods is forbidden: User \"viewer\" cannot list resource \"pods\" at the cluster scope (HTTP 403). No pod is loaded.",
        "Pods: pause after 3 failures: the list of the pods got no answer in 15s; next attempt in 2 s. Its rows are old since 07:59:12.",
        "the connect did not start: the cluster entry `lab` has no certificate authority",
    ];
    for text in errors {
        assert!(shows(&window, text), "{text}");
    }
    assert!(shows(&window, "12 pods"), "the two clusters with pods");
    let rows: Vec<(String, String)> = column(&window, "Cluster").into_iter().zip(column(&window, "Name")).collect();
    let of = |cluster: &str| rows.iter().filter(|(of, _)| of == cluster).count();
    assert_eq!((of("ams-prod"), of("sgp-prod")), (6, 6), "a cluster with an error keeps its rows: {rows:?}");
    assert_eq!(marks(&window, MARK), 6, "only the rows of the cluster that is not live have the mark");
}

/// Weakness 7: a watch that stops with no message. The objects do not change at that
/// moment, so the view must show the new state with no new rows.
#[test]
fn w7_a_watch_that_stops_shows_in_the_next_frame_as_a_note_with_the_reason_and_a_mark_on_each_row() {
    let live_store = store(StoreState::Live, &pods());
    let mut window = window(view(vec![cluster("ams-prod", in_state(&live_store, StoreState::Live))]));
    let rows = column(&window, "Name").len();
    assert_eq!((marks(&window, MARK), window.query_all_by_label_contains("Pods: ").count()), (0, 0));

    let stops = [
        (
            StoreState::Backoff { failures: 1, reason: no_answer(), until: now() + SECOND },
            "Pods: pause after 1 failures: the list of the pods got no answer in 15s; next attempt in 1 s. Its rows are old since 07:59:12.",
        ),
        (
            StoreState::InternalError { reason: "the task of the store panicked: a defect".into() },
            "Pods: internal error: the task of the store panicked: a defect. Its rows are old since 07:59:12.",
        ),
        (StoreState::Stopped, "Pods: stopped. Its rows are old since 07:59:12."),
    ];
    for (state, note) in stops {
        window.state_mut().view = view(vec![cluster("ams-prod", in_state(&live_store, state))]);
        // One frame: no worker computes for a new state of the same objects.
        window.step();
        assert!(shows(&window, note), "{note}");
        // The note takes the place of the last rows of the frame before.
        let marked = marks(&window, MARK);
        assert!(marked + 2 >= rows && marked == column(&window, "Name").len(), "{marked} of {rows} rows");

        window.state_mut().view = view(vec![cluster("ams-prod", in_state(&live_store, StoreState::Live))]);
        window.step();
        assert_eq!((marks(&window, MARK), window.query_all_by_label_contains("Pods: ").count()), (0, 0), "live again");
    }
}

/// Weakness 8: old data that shows as current. The connection of the cluster is silent,
/// and the last data of the store is 48 s old.
#[test]
fn w8_each_row_of_a_cluster_with_no_connection_shows_old_since_and_the_other_cluster_does_not() {
    let silent = ChannelStatus {
        last_error: Some("the server sent no ping answer in 5s".to_owned()),
        last_frame_age: Some(48 * SECOND),
        ..channel(ChannelState::Backoff { failures: 1, pause: SECOND })
    };
    let lost = connected("sgp-prod", status([silent.clone(), silent.clone(), silent], None));
    let waits = data(store(StoreState::Waiting(Wait::Channel), &pods_of("sgp")));
    let mut window = window(view(vec![lost.with(Some(waits)), live("ams-prod", &pods_of("ams"))]));

    let note =
        "Pods: waits for a connection: watches: the server sent no ping answer in 5s. Its rows are old since 07:59:12.";
    assert!(shows(&window, note) && shows(&window, "sgp-prod: no connection"));
    let rows: Vec<(String, String)> = column(&window, "Cluster").into_iter().zip(column(&window, "Name")).collect();
    let old = rows.iter().filter(|(cluster, _)| cluster == "sgp-prod").count();
    assert_eq!((old, marks(&window, MARK)), (6, 6), "{rows:?}");

    // The panel of an old pod tells the same, and the panel of a current pod does not.
    window.get_by_label("sgp-crash").click();
    window.run();
    assert!(shows(&window, "The values are old since 07:59:12."));
    window.get_by_label("ams-crash").click();
    window.run();
    assert!(shows(&window, "Close the details") && !shows(&window, "The values are old since 07:59:12."));
}
