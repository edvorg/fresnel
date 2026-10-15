//! The views of the Cluster kinds: a Node with its pods, a Namespace with its quotas, its
//! limit ranges and the counts of its objects, and an Event with its object.

use std::collections::BTreeSet;
use std::sync::Arc;

use fresnel_core::app::{ClusterRow, Command};
use fresnel_core::kinds::Kind;
use fresnel_core::search::Source;
use fresnel_core::store::StoreState;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Event, LimitRange, Namespace, Node, ResourceQuota, Service};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use super::{and, follow, shows_part};
use crate::data::{changed_object, events, held, in_state, of_kinds, pods, scene, store};
use crate::fake::{SECOND, now, until};
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
/// `scene`, and the stores that are always on: the pods, the Deployments, and the Services
/// in a pause.
pub(super) fn namespaces() -> ClusterRow {
    let quotas: Vec<Arc<ResourceQuota>> = of_kinds::<ResourceQuota>().iter().map(|quota| in_scene(&**quota)).collect();
    let ranges: Vec<Arc<LimitRange>> = of_kinds::<LimitRange>().iter().map(|range| in_scene(&**range)).collect();
    let row = and(and(live("ams-prod", &scene::<Namespace>()), &quotas), &ranges);
    let mut data = row.data.clone().expect("data");
    let pause = StoreState::Backoff { failures: 2, reason: no_answer(), until: now() + 2 * SECOND };
    let services = in_state(&store(StoreState::Live, &scene::<Service>()), pause);
    data.sources = Arc::from([
        Source::full(held(store(StoreState::Live, &pods()))),
        Source::full(held(store(StoreState::Live, &scene::<Deployment>()))),
        Source::full(held(services)),
    ]);
    ClusterRow { data: Some(data), ..row }
}

#[test]
fn a_namespace_shows_its_quotas_limit_ranges_and_the_counts_of_its_objects() {
    let mut window = opened::<Namespace>(vec![namespaces()], "scene");
    let wanted = BTreeSet::from([Kind::Namespace, Kind::ResourceQuota, Kind::LimitRange]);
    assert!(window.state().sent.contains(&Command::ShowKinds(wanted)));
    let texts =
        ["Status", "Active", "RESOURCE QUOTAS", "ResourceQuota quota", "30%", "LIMIT RANGES", "LimitRange limits"];
    let texts = [&texts[..], &["16Mi", "OBJECTS", "Pods", "23", "Deployments", "2", "Services", "6"]].concat();
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    assert!(shows_part(&window, "old since 07:59:12"), "the count of a store that is not live");

    assert!(!in_panel(&window, "Waits for"), "the finalizer of each namespace matters only at its end");
    follow(&mut window, "ResourceQuota quota");
    until(&mut window, "the panel of the quota", |window| in_panel(window, "USE"));

    let window = opened::<Namespace>(vec![namespaces()], "scene-ending");
    assert_eq!(missing(&window, &["Terminating", "Waits for", "kubernetes"]), Vec::<String>::new());
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
    follow(&mut window, "Pod crash");
    until(&mut window, "the panel of the pod", |window| in_panel(window, "CONTAINERS") && in_panel(window, "crash"));
}
