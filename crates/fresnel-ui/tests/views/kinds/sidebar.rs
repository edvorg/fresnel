//! The sidebar: the groups and the kinds of plan section 11.3 with their counts, and the
//! kinds that the core holds for the open page.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::{ClusterRow, Command};
use fresnel_core::cluster::LinkState;
use fresnel_core::kinds::{Kind, Sidebar};
use fresnel_core::store::{StoreState, StoreStatus};
use k8s_openapi::api::coordination::v1::Lease;

use super::scene::{deployments, live, with};
use crate::data::{of_kinds, store};
use crate::fake::{Window, error, now, off, open_page, row, view, window};
use crate::pods::scenes::{forbidden, no_answer};

/// The facts of the stores of a cluster with these counts of objects, each store live.
fn with_counts(row: ClusterRow, counts: &[(Kind, usize)]) -> ClusterRow {
    let stores: Vec<StoreStatus> = counts.iter().map(|(kind, items)| status(*kind, *items, StoreState::Live)).collect();
    with_stores(row, stores)
}

fn with_stores(mut row: ClusterRow, stores: Vec<StoreStatus>) -> ClusterRow {
    if let Some(data) = row.data.as_mut() {
        data.stores = stores.into();
    }
    row
}

/// The facts of a store in this state. A forbidden store never had a complete list.
fn status(kind: Kind, items: usize, state: StoreState) -> StoreStatus {
    let listed = !matches!(state, StoreState::Forbidden { .. });
    StoreStatus { kind, state, since: now(), last_contact: Some(now()), listed, items, dropped: 0, generation: 1 }
}

/// The count of an entry of the sidebar, as a screen reader reads it.
fn count(window: &Window, title: &str) -> Option<String> {
    window.get_by_role_and_label(Role::RadioButton, title).accesskit_node().value().map(|value| value.to_owned())
}

#[test]
fn the_sidebar_has_each_kind_in_its_group_with_the_count_of_the_selected_clusters() {
    let counts = [(Kind::Pod, 25), (Kind::Deployment, 4), (Kind::ConfigMap, 2), (Kind::Secret, 3)];
    let leases: Vec<Arc<Lease>> = of_kinds();
    let ams = with_counts(with("ams-prod", store(StoreState::Live, &leases)), &counts);
    let fra = with_counts(live("fra-stage", &deployments()), &counts[..2]);
    let mut not_selected = with_counts(live("lab", &deployments()), &counts);
    not_selected.selected = false;
    let window = window(view(vec![ams, fra, not_selected, off("nyc-prod")]));

    for group in Sidebar::ALL {
        let title = group.title().to_uppercase();
        assert!(window.query_by_label(&title).is_some(), "the group {title}");
        for kind in group.kinds() {
            assert!(window.query_by_role_and_label(Role::RadioButton, kind.info().title).is_some(), "{kind:?}");
        }
    }
    assert_eq!(window.query_all_by_role(Role::RadioButton).count(), Kind::ALL.len() + 1, "the kinds and Connections");
    assert_eq!(count(&window, "Pods").as_deref(), Some("50"), "two selected clusters");
    assert_eq!(count(&window, "Deployments").as_deref(), Some("8"));
    let some = "a selected cluster has no store of the kind, so the count is not complete";
    assert_eq!(count(&window, "ConfigMaps").as_deref(), Some("2, not current"), "{some}");
    assert_eq!(count(&window, "Nodes"), None, "no store of the kind");
    // A kind of tier C has a count only while a page holds its store.
    assert_eq!(count(&window, "Leases"), Some(format!("{}, not current", leases.len())), "{some}");
    assert_eq!(count(&window, "Endpoints"), None);
}

/// Design rules 4 and 5: a count of a store that is not live can be old, and a forbidden
/// store has no object. Such a count has the freshness mark, so its "0" never looks like
/// a kind with no object.
#[test]
fn a_count_of_a_store_that_is_not_live_or_of_a_cluster_with_no_stores_is_not_current() {
    let pause = StoreState::Backoff { failures: 2, reason: no_answer(), until: now() };
    let ams =
        with_counts(live("ams-prod", &deployments()), &[(Kind::Pod, 25), (Kind::Deployment, 4), (Kind::Secret, 3)]);
    let nyc = with_stores(
        live("nyc-prod", &deployments()),
        vec![
            status(Kind::Pod, 25, pause),
            status(Kind::Deployment, 4, StoreState::Live),
            status(Kind::Secret, 0, StoreState::Forbidden { reason: forbidden() }),
        ],
    );
    let mut window = window(view(vec![ams.clone(), nyc]));
    assert_eq!(count(&window, "Deployments").as_deref(), Some("8"), "each store is live");
    assert_eq!(count(&window, "Pods").as_deref(), Some("50, not current"), "a store in a pause");
    assert_eq!(count(&window, "Secrets").as_deref(), Some("3, not current"), "a forbidden store");

    // A selected cluster whose connect failed has no stores: no count of it is known.
    let failed = row("lab", LinkState::Failed(error("the cluster entry `lab` has no certificate authority")));
    window.state_mut().view = view(vec![ams.clone(), failed]);
    window.run();
    assert_eq!(count(&window, "Deployments").as_deref(), Some("4, not current"));
    window.state_mut().view = view(vec![ams]);
    window.run();
    assert_eq!(count(&window, "Deployments").as_deref(), Some("4"), "current again");
}

#[test]
fn a_page_names_its_kind_to_the_core_and_the_connections_page_keeps_it() {
    let mut window = window(view(vec![live("ams-prod", &deployments())]));
    let kinds = |kinds: &[Kind]| Command::ShowKinds(kinds.iter().copied().collect::<BTreeSet<Kind>>());
    assert_eq!(window.state().sent, [], "the pods need no store of their own");
    open_page(&mut window, "Deployments");
    open_page(&mut window, "Connections");
    open_page(&mut window, "Deployments");
    assert_eq!(window.state().sent, [kinds(&[Kind::Deployment])], "the store stays during the Connections page");
    open_page(&mut window, "ConfigMaps");
    open_page(&mut window, "Pods");
    assert_eq!(window.state().sent[1..], [kinds(&[Kind::ConfigMap]), kinds(&[])]);
}
