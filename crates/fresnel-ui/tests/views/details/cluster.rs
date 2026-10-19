//! The views of the Cluster kinds: a Node with its pods, a Namespace with its quotas, its
//! limit ranges and the counts of its objects, and an Event with its object.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::{ClusterRow, Command};
use fresnel_core::kinds::Kind;
use fresnel_core::search::Source;
use fresnel_core::store::{Snapshot, StoreState};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Event, LimitRange, Namespace, Node, ResourceQuota, Service};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use super::{and, follow, shows_part};
use crate::data::{changed_object, events, held, in_state, of_kinds, pods, scene, store};
use crate::fake::{SECOND, now, shows, until};
use crate::kinds::scene::{live, nodes};
use crate::kinds::{in_panel, missing, times_in_panel};
use crate::pods::scenes::no_answer;
use crate::workloads::scene::opened;

#[test]
fn a_node_shows_its_addresses_capacity_and_pods_with_their_namespace() {
    let node = nodes()[0].metadata.name.clone().expect("a name");
    let mut window = opened::<Node>(vec![live("ams-prod", &nodes())], &node);
    let texts =
        ["New pods", "yes", "Pod ranges", "10.42.0.0/24", "Provider ID", "CAPACITY", "Allocatable", "cpu", "110"];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    assert!(shows_part(&window, "InternalIP 10.199.146.2"), "the addresses");
    assert!(in_panel(&window, "PODS") && times_in_panel(&window, "scene") >= 10, "the namespace of each pod");
    let crash = "crash";
    follow(&mut window, crash);
    until(&mut window, "the panel of the pod", |window| in_panel(window, "CONTAINERS"));

    let cordoned = changed_object(&*nodes()[0], |value| {
        value["spec"]["unschedulable"] = true.into();
        value["spec"]["taints"] = json!([{"key": "node.kubernetes.io/unschedulable", "effect": "NoSchedule",
            "timeAdded": "2027-01-15T07:58:00Z"}]);
    });
    let window = opened::<Node>(vec![live("ams-prod", &[cordoned])], &node);
    assert!(shows_part(&window, "no: the node is cordoned"));
    let taint = "node.kubernetes.io/unschedulable:NoSchedule since 2027-01-15 07:58:00 (2 min 0 s ago)";
    assert!(shows_part(&window, taint));
}

/// An object of the kinds test, moved into the namespace of the pods.
fn in_scene<K: Serialize + DeserializeOwned>(object: &K) -> Arc<K> {
    changed_object(object, |value| value["metadata"]["namespace"] = "scene".into())
}

/// The cluster of the Namespaces, with the quotas and the limit ranges of the namespace
/// `scene`, and the stores that are always on: the pods, the Deployments, the Services in a
/// pause, and the Events of a store at its limit.
pub(super) fn namespaces(cluster: &str) -> ClusterRow {
    let quotas: Vec<Arc<ResourceQuota>> = of_kinds::<ResourceQuota>().iter().map(|quota| in_scene(&**quota)).collect();
    let ranges: Vec<Arc<LimitRange>> = of_kinds::<LimitRange>().iter().map(|range| in_scene(&**range)).collect();
    let row = and(and(live(cluster, &scene::<Namespace>()), &quotas), &ranges);
    let mut data = row.data.clone().expect("data");
    let pause = StoreState::Backoff { failures: 2, reason: no_answer(), until: now() + 2 * SECOND };
    let services = in_state(&store(StoreState::Live, &scene::<Service>()), pause);
    let full = Snapshot { dropped: 12_000, ..store(StoreState::Live, &events()) };
    data.sources = Arc::from([
        Source::full(held(store(StoreState::Live, &pods()))),
        Source::full(held(store(StoreState::Live, &scene::<Deployment>()))),
        Source::full(held(services)),
        Source::full(held(full)),
    ]);
    ClusterRow { data: Some(data), ..row }
}

#[test]
fn a_namespace_shows_its_quotas_limit_ranges_and_the_counts_of_its_objects() {
    let mut window = opened::<Namespace>(vec![namespaces("ams-prod")], "scene");
    let wanted = BTreeSet::from([Kind::Namespace, Kind::ResourceQuota, Kind::LimitRange]);
    assert!(window.state().sent.contains(&Command::ShowKinds(wanted)));
    let texts =
        ["Status", "Active", "RESOURCE QUOTAS", "ResourceQuota quota", "30%", "LIMIT RANGES", "LimitRange limits"];
    let texts = [&texts[..], &["16Mi", "OBJECTS", "Pods", "23", "Deployments", "2", "Services", "6"]].concat();
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    assert!(shows_part(&window, "old since 07:59:12"), "the count of a store that is not live");
    let limit = "or more: 12,000 older ones left the store at its limit";
    assert!(in_panel(&window, "156") && shows_part(&window, limit), "the count of the Events is too small");

    assert!(!in_panel(&window, "Waits for"), "the finalizer of each namespace matters only at its end");
    follow(&mut window, "ResourceQuota quota");
    until(&mut window, "the panel of the quota", |window| in_panel(window, "USE"));

    let window = opened::<Namespace>(vec![namespaces("ams-prod")], "scene-ending");
    assert_eq!(missing(&window, &["Terminating", "Waits for", "kubernetes"]), Vec::<String>::new());
}

/// Weakness 2 of plan section 6: a namespace of one name in two clusters. The panel of the
/// cluster with no quota shows none, and no quota or count of the other cluster.
#[test]
fn a_namespace_of_one_name_in_two_clusters_shows_the_quotas_and_counts_of_its_own_cluster() {
    let bare = live("ams-prod", &scene::<Namespace>());
    let bare = and(and(bare, &Vec::<Arc<ResourceQuota>>::new()), &Vec::<Arc<LimitRange>>::new());
    let window = opened::<Namespace>(vec![bare, namespaces("fra-prod")], "scene");
    assert!(in_panel(&window, "ams-prod") && !in_panel(&window, "fra-prod"), "the panel of the first cluster");
    assert_eq!(times_in_panel(&window, "None."), 2, "no quota and no limit range");
    assert!(!in_panel(&window, "ResourceQuota quota") && !in_panel(&window, "Deployments"), "of the other cluster");
}

#[test]
fn an_event_shows_its_object_as_a_link_and_no_events_of_its_own() {
    let mut window = opened::<Event>(vec![live("ams-prod", &events())], "crash.18de28919fa8e1d0");
    let texts = ["Type", "Warning", "Reason", "BackOff", "Object", "Pod crash", "Source", "kubelet, 35f62a67b541"];
    assert_eq!(
        missing(&window, &[&texts[..], &["Count", "3", "First seen", "Last seen"]].concat()),
        Vec::<String>::new()
    );
    assert!(!in_panel(&window, "EVENTS"), "an Event has no Events");
    // The state of its problem now, from the live pod, in the panel and in the Now column.
    let now = ["Now", "ongoing", "the container crash waits: CrashLoopBackOff"];
    assert_eq!(missing(&window, &now), Vec::<String>::new());
    // A field has no space between its parts: the state has its own.
    let why = window.get_by_label(now[2]).rect();
    let in_line = |chip: &egui::Rect| (chip.top() - why.top()).abs() < 4.0 && chip.left() < why.left();
    let chip = window.query_all_by_label("ongoing").map(|node| node.rect()).find(in_line);
    let gap = chip.map(|chip| why.left() - chip.right());
    assert!(gap.is_some_and(|gap| gap >= 4.0), "a space between the state and its fact: {gap:?}");
    let cell = "Ongoing now: the container crash waits: CrashLoopBackOff.";
    assert!(shows(&window, cell) && times_in_panel(&window, cell) == 0, "the cell of its row in the table");
    follow(&mut window, "Pod crash");
    until(&mut window, "the panel of the pod", |window| in_panel(window, "CONTAINERS") && in_panel(window, "crash"));
}
