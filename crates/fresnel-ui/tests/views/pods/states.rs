//! The Pods view for each state of a Pod store (design rules 4 and 5): the rows stay,
//! each row has its freshness mark, and a note tells the state with its reason.

use egui_kittest::kittest::Queryable as _;
use fresnel_core::CoreError;
use fresnel_core::cluster::LinkState;
use fresnel_core::net::credential::{CredentialFailure, CredentialState};
use fresnel_core::net::{ChannelState, ChannelStatus, ErrorClass};
use fresnel_core::store::{Snapshot, StoreState, Wait};

use crate::data::{cluster, data, empty_store, live, pods, store};
use crate::fake::{
    SECOND, Window, WithData as _, channel, connected, error, new_channel, now, off, row, shows, status, view, window,
};
use crate::pods::scenes::{forbidden, no_answer};
use crate::pods::{column, marks};

/// The mark of a row from a store whose last data arrived 48 s before now.
const MARK: &str = "old since 07:59:12";
const OLD: &str = "Its rows are old since 07:59:12.";

#[test]
fn a_live_store_shows_its_pods_with_no_note_and_no_mark() {
    let window = window(view(vec![live("ams-prod", &pods())]));
    assert!(shows(&window, "25 pods"));
    let names = column(&window, "Name");
    assert_eq!(names[..4], ["agent-8djvd", "broken-6bf768c954-gjvsw", "coredns-64fd4b4794-6lsjp", "crash"]);
    assert!(window_has_no_note(&window) && marks(&window, MARK) == 0);
}

/// True when no cluster has a note above the table.
fn window_has_no_note(window: &Window) -> bool {
    let notes = ["Pods: ", "the connect "];
    notes.iter().all(|start| window.query_all_by_label_contains(start).next().is_none())
}

/// In each state but "live", the store sends no change of the cluster to the view. The
/// rows of the store stay, as old rows.
#[test]
fn each_state_of_a_store_that_is_not_live_has_its_note_and_marks_each_row_and_keeps_the_rows() {
    let live_names = column(&window(view(vec![live("ams-prod", &pods())])), "Name");
    let not_found = CoreError::new(ErrorClass::Invalid, "the server could not find the requested resource (HTTP 404)");
    let states = [
        (StoreState::Waiting(Wait::ListSlot), "waits for a list slot".to_owned()),
        (StoreState::Waiting(Wait::Stream), "waits for the start of its watch".to_owned()),
        (StoreState::Listing { loaded: 500 }, "list in progress, 500 objects".to_owned()),
        (
            StoreState::Attempt { failures: 2, reason: no_answer() },
            "attempt after 2 failures: the list of the pods got no answer in 15s".to_owned(),
        ),
        (
            StoreState::Backoff { failures: 3, reason: no_answer(), until: now() + 2 * SECOND },
            "pause after 3 failures: the list of the pods got no answer in 15s; next attempt in 2 s".to_owned(),
        ),
        (StoreState::Throttled { until: now() + 30 * SECOND }, "throttled by the server until 08:00:30".to_owned()),
        (StoreState::Forbidden { reason: forbidden() }, format!("forbidden: {}", forbidden())),
        (StoreState::NotSupported { reason: not_found.clone() }, format!("not supported: {not_found}")),
        (
            StoreState::InternalError { reason: "the task of the store panicked: index 4 of 3".into() },
            "internal error: the task of the store panicked: index 4 of 3".to_owned(),
        ),
        (StoreState::Stopped, "stopped".to_owned()),
    ];
    for (state, words) in states {
        let window = window(view(vec![cluster("ams-prod", store(state.clone(), &pods()))]));
        let note = format!("Pods: {words}. {OLD}");
        assert!(shows(&window, &note), "{state:?}: the note `{note}`");
        let names = column(&window, "Name");
        // The note takes the place of one row.
        assert!(names.len() > 10 && live_names.starts_with(&names), "{state:?}: the rows stay (design rule 5)");
        assert_eq!(marks(&window, MARK), names.len(), "{state:?}: each row has its mark (design rule 4)");
        assert!(shows(&window, "25 pods"), "{state:?}");
    }
}

/// A store that waits for its cluster has the reason of the cluster in its note.
#[test]
fn a_store_that_waits_for_its_cluster_tells_the_reason_of_the_cluster() {
    let lost = ChannelStatus {
        last_error: Some("connect: no answer in 10s".to_owned()),
        ..channel(ChannelState::Backoff { failures: 3, pause: 2 * SECOND })
    };
    let no_connection = status([lost.clone(), lost.clone(), lost], None);
    let refused =
        CredentialState::Failed { failure: CredentialFailure::Refused, failures: 2, next_attempt: now() + 8 * SECOND };
    let no_credential = status([(); 3].map(|()| channel(ChannelState::Live)), Some(refused));
    let waits = |wait| Some(data(store(StoreState::Waiting(wait), &pods())));
    let clusters = vec![
        connected("sgp-prod", no_connection).with(waits(Wait::Channel)),
        connected("nyc-prod", no_credential).with(waits(Wait::Credential)),
    ];
    let window = window(view(clusters));
    let texts = [
        format!("Pods: waits for a connection: watches: connect: no answer in 10s. {OLD}"),
        format!(
            "Pods: waits for a credential: the server refused a new token of the credential plugin (HTTP 401). {OLD}"
        ),
        "sgp-prod: no connection".to_owned(),
        "nyc-prod: no credential".to_owned(),
    ];
    for text in &texts {
        assert!(shows(&window, text), "{text}");
    }
    assert_eq!(marks(&window, MARK), column(&window, "Name").len(), "each row of the two clusters is old");
}

/// Design rule 5: no empty list stands for an error. Only a store with a complete list
/// knows that a cluster has no pod.
#[test]
fn a_table_with_no_row_tells_why_and_never_says_no_pod_for_a_list_that_did_not_arrive() {
    const NOT_LOADED: &str = "The pods are not loaded. The notes above give the state of each cluster.";
    const NONE: &str = "The selected clusters have no pod.";
    let connecting = || status([(); 3].map(|()| new_channel(ChannelState::Connecting { attempt: 1 })), None);
    let first_list = empty_store(StoreState::Listing { loaded: 0 }, now() - SECOND);
    let waiting = empty_store(StoreState::Waiting(Wait::Channel), now() - SECOND);
    let refused = empty_store(StoreState::Forbidden { reason: forbidden() }, now() - SECOND);
    let cases = [
        (vec![live("ams-prod", &[])], NONE, None),
        (
            vec![cluster("ams-prod", first_list)],
            NOT_LOADED,
            Some("Pods: list in progress, 0 objects. No pod is loaded."),
        ),
        (
            vec![connected("ams-prod", connecting()).with(Some(data(waiting)))],
            NOT_LOADED,
            Some("Pods: waits for a connection. No pod is loaded."),
        ),
        (
            vec![cluster("ams-prod", refused)],
            NOT_LOADED,
            Some(
                "Pods: forbidden: pods is forbidden: User \"viewer\" cannot list resource \"pods\" at the cluster scope (HTTP 403). No pod is loaded.",
            ),
        ),
        (
            vec![row("lab", LinkState::Failed(error("no server")))],
            NOT_LOADED,
            Some("the connect did not start: no server"),
        ),
        (vec![row("lab", LinkState::Off)], NOT_LOADED, Some("the connect starts")),
        // One cluster has no pod, and the list of the other one did not arrive.
        (vec![live("ams-prod", &[]), row("lab", LinkState::Off)], NOT_LOADED, Some("the connect starts")),
        (vec![off("ams-prod")], "No cluster is selected. Select a cluster in the top bar.", None),
    ];
    for (clusters, text, note) in cases {
        let window = window(view(clusters));
        assert!(shows(&window, text), "`{text}` for {:?}", window.state().view.clusters);
        assert!(text == NONE || !shows(&window, NONE), "no list arrived, so no text says that no pod exists");
        match note {
            Some(note) => assert!(shows(&window, note), "{note}"),
            None => assert!(window_has_no_note(&window)),
        }
        assert!(shows(&window, "0 pods"));
    }
}

/// The first list of a large cluster takes time. The pods of its first pages show, with
/// the mark and the note that the list is not complete.
#[test]
fn the_pods_of_a_first_list_in_progress_show_as_not_complete() {
    let partial = Snapshot { listed: false, ..store(StoreState::Listing { loaded: 25 }, &pods()) };
    let window = window(view(vec![cluster("ams-prod", partial)]));
    assert!(shows(&window, "Pods: list in progress, 25 objects. Its first list is not complete."));
    assert_eq!(marks(&window, MARK), column(&window, "Name").len());
    assert!(shows(&window, "25 pods"));
}
