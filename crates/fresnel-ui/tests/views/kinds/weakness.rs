//! The weaknesses of Headlamp of plan section 6 that apply to the table of a kind, for the
//! kind of each group of the sidebar. Each test has the number of its line in that section.

use std::sync::Arc;

use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::ClusterRow;
use fresnel_core::cluster::LinkState;
use fresnel_core::kinds::Describe;
use fresnel_core::net::{ChannelState, ChannelStatus};
use fresnel_core::store::{StoreState, Wait};

use super::in_panel;
use super::scene::{config_maps, deployments, live, nodes, revisions, service_accounts, services, with};
use crate::data::{empty_store, in_state, store};
use crate::fake::{SECOND, Window, channel, connected, error, now, open_page, row, shows, status, until, view, window};
use crate::pods::scenes::{forbidden, no_answer};
use crate::pods::{column, marks};

const MARK: &str = "old since 07:59:12";

/// Runs a check for the kind of each group of the sidebar.
macro_rules! each_group {
    ($check:ident) => {
        $check(nodes());
        $check(deployments());
        $check(config_maps());
        $check(services());
        $check(service_accounts());
        $check(revisions());
    };
}

/// "1 Node", "16 Services": the count of the table of a kind.
pub(crate) fn count_of<K: Describe>(count: usize) -> String {
    if count == 1 { format!("1 {}", K::KIND) } else { format!("{count} {}", K::TITLE) }
}

fn name_of<K: Describe>(object: &K) -> String {
    object.metadata().name.clone().expect("a name")
}

/// The window with these clusters and the page of the kind, after its first rows.
pub(crate) fn page<K: Describe>(clusters: Vec<ClusterRow>, rows: usize) -> Window {
    let mut window = window(view(clusters));
    open_page(&mut window, K::TITLE);
    let count = count_of::<K>(rows);
    until(&mut window, &format!("the rows of the {}", K::TITLE), |window| shows(window, &count));
    window
}

/// Weakness 1: data from the first selected cluster only.
#[test]
fn w1_the_table_of_each_kind_has_the_objects_of_each_selected_cluster() {
    fn check<K: Describe>(objects: Vec<Arc<K>>) {
        let n = objects.len();
        let mut window = page::<K>(vec![live("ams-prod", &objects), live("fra-stage", &objects)], 2 * n);
        let rows: Vec<(String, String)> = column(&window, "Name").into_iter().zip(column(&window, "Cluster")).collect();
        for object in &objects {
            let name = name_of(&**object);
            for cluster in ["ams-prod", "fra-stage"] {
                let row = (name.clone(), cluster.to_owned());
                assert!(rows.contains(&row), "{}: the row {row:?} in {rows:?}", K::TITLE);
            }
        }
        // A row is an object of one cluster: a click on the second row of a name opens
        // the object of the second cluster.
        let name = name_of(&*objects[0]);
        window.query_all_by_label(&name).nth(1).expect("the row of the second cluster").click();
        window.run();
        assert!(in_panel(&window, "fra-stage") && !in_panel(&window, "ams-prod"), "{}: the panel", K::TITLE);

        // A cluster that leaves the selection takes its rows with it, and its panel.
        window.state_mut().view = view(vec![live("ams-prod", &objects)]);
        until(&mut window, "the rows of one cluster", |window| shows(window, &count_of::<K>(n)));
        assert!(!shows(&window, "Close the details"), "{}", K::TITLE);
    }
    each_group!(check);
}

/// Weakness 3: only the first error is kept.
#[test]
fn w3_each_cluster_that_fails_shows_its_own_error_and_the_other_clusters_keep_their_rows() {
    fn check<K: Describe>(objects: Vec<Arc<K>>) {
        let (n, title, one) = (objects.len(), K::TITLE, K::KIND);
        let pause = StoreState::Backoff { failures: 3, reason: no_answer(), until: now() + 2 * SECOND };
        let clusters = vec![
            with::<K>("nyc-prod", empty_store(StoreState::Forbidden { reason: forbidden() }, now() - 20 * SECOND)),
            with("sgp-prod", store(pause, &objects)),
            row("lab", LinkState::Failed(error("the cluster entry `lab` has no certificate authority"))),
            live("ams-prod", &objects),
        ];
        let window = page::<K>(clusters, 2 * n);
        let errors = [
            format!("{title}: forbidden: {}. No {one} is loaded.", forbidden()),
            format!(
                "{title}: pause after 3 failures: {}; next attempt in 2 s. Its rows are old since 07:59:12.",
                no_answer()
            ),
            "the connect did not start: the cluster entry `lab` has no certificate authority".to_owned(),
        ];
        for text in errors {
            assert!(shows(&window, &text), "{text}");
        }
        let clusters = column(&window, "Cluster");
        let of = |cluster: &str| clusters.iter().filter(|of| *of == cluster).count();
        assert_eq!((of("ams-prod"), of("sgp-prod")), (n, n), "{title}: a cluster with an error keeps its rows");
        assert_eq!(marks(&window, MARK), n, "{title}: only the rows of the cluster that is not live have the mark");
    }
    each_group!(check);
}

/// Weakness 7: a watch that stops with no message. The objects do not change at that
/// moment, so the page must show the new state with no new rows.
#[test]
fn w7_a_watch_that_stops_shows_in_the_next_frame_as_a_note_and_a_mark_on_each_row() {
    fn check<K: Describe>(objects: Vec<Arc<K>>) {
        let (n, title) = (objects.len(), K::TITLE);
        let live_store = store(StoreState::Live, &objects);
        let mut window = page::<K>(vec![with("ams-prod", in_state(&live_store, StoreState::Live))], n);
        let notes = |window: &Window| window.query_all_by_label_contains(&format!("{title}: ")).count();
        assert_eq!((marks(&window, MARK), notes(&window)), (0, 0));
        let stops = [
            (
                StoreState::Backoff { failures: 1, reason: no_answer(), until: now() + SECOND },
                format!("pause after 1 failures: {}; next attempt in 1 s", no_answer()),
            ),
            (
                StoreState::InternalError { reason: "the task of the store panicked: a defect".into() },
                "internal error: the task of the store panicked: a defect".to_owned(),
            ),
            (StoreState::Stopped, "stopped".to_owned()),
        ];
        for (state, words) in stops {
            window.state_mut().view = view(vec![with("ams-prod", in_state(&live_store, state))]);
            // One frame: no worker computes for a new state of the same objects.
            window.step();
            let note = format!("{title}: {words}. Its rows are old since 07:59:12.");
            assert!(shows(&window, &note), "{note}");
            assert_eq!(marks(&window, MARK), column(&window, "Name").len(), "{title}: each row has its mark");

            window.state_mut().view = view(vec![with("ams-prod", in_state(&live_store, StoreState::Live))]);
            window.step();
            assert_eq!((marks(&window, MARK), notes(&window)), (0, 0), "{title}: live again");
        }
    }
    each_group!(check);
}

/// Weakness 8: old data that shows as current. The connection of a cluster is silent, and
/// the last data of its store is 48 s old.
#[test]
fn w8_each_row_of_a_cluster_with_no_connection_shows_old_since_and_the_other_cluster_does_not() {
    fn check<K: Describe>(objects: Vec<Arc<K>>) {
        let (n, title) = (objects.len(), K::TITLE);
        let silent = ChannelStatus {
            last_error: Some("the server sent no ping answer in 5s".to_owned()),
            last_frame_age: Some(48 * SECOND),
            ..channel(ChannelState::Backoff { failures: 1, pause: SECOND })
        };
        let waits = with("sgp-prod", store(StoreState::Waiting(Wait::Channel), &objects)).data;
        let lost =
            ClusterRow { data: waits, ..connected("sgp-prod", status([silent.clone(), silent.clone(), silent], None)) };
        let mut window = page::<K>(vec![lost, live("ams-prod", &objects)], 2 * n);

        let note = format!(
            "{title}: waits for a connection: watches: the server sent no ping answer in 5s. Its rows are old since 07:59:12."
        );
        assert!(shows(&window, &note) && shows(&window, "sgp-prod: no connection"), "{note}");
        let clusters = column(&window, "Cluster");
        let old = clusters.iter().filter(|cluster| *cluster == "sgp-prod").count();
        assert_eq!((old, marks(&window, MARK)), (n, n), "{title}: {clusters:?}");

        // The panel of an old object tells the same, and the panel of a current one does not.
        let name = name_of(&*objects[0]);
        let rows = column(&window, "Name");
        let place =
            |cluster: &str| rows.iter().zip(&clusters).filter(|(of, _)| **of == name).position(|(_, at)| at == cluster);
        let (old_at, live_at) = (place("sgp-prod").expect("an old row"), place("ams-prod").expect("a row"));
        window.query_all_by_label(&name).nth(old_at).expect("the old row").click();
        window.run();
        assert!(shows(&window, "The values are old since 07:59:12."), "{title}");
        // The panel has the name too, so it closes before the next click.
        window.get_by_label("Close the details").click();
        window.run();
        window.query_all_by_label(&name).nth(live_at).expect("the current row").click();
        window.run();
        assert!(
            shows(&window, "Close the details") && !shows(&window, "The values are old since 07:59:12."),
            "{title}"
        );
    }
    each_group!(check);
}
