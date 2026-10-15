use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, SystemTime};

use fresnel_core::metrics::{MetricsSettings, NodeLoad, Sample, SeriesKey};
use fresnel_core::store::StoreState;
use k8s_openapi::api::apps::v1::Deployment;
use serde_json::json;

use super::*;
use crate::testing::{TestTasks, fixture, fixture_pods, snapshot};

/// The node of the scene.
const NODE: &str = "35f62a67b541";
const MI: u64 = 1024 * 1024;

fn metrics(bed: &TestTasks, requests: Amount) -> Held<Metrics> {
    let mut metrics = Metrics::waiting(ClusterId::new("ams"), Duration::from_secs(15), SystemTime::UNIX_EPOCH);
    let loads = BTreeMap::from([(Arc::from(NODE), NodeLoad { requests, pods: 3 })]);
    let sample = Sample { at: 1, cover: fresnel_core::metrics::Cover::ONE, usage: Amount::default() };
    metrics.take_samples(&[(SeriesKey::Cluster, sample)], loads, SystemTime::UNIX_EPOCH, &MetricsSettings::default());
    bed.held(metrics)
}

/// A cluster of the scene with these kinds besides the pods.
fn input(bed: &TestTasks, objects: Objects) -> Input {
    let pods = bed.held(snapshot(&fixture_pods(), StoreState::Live));
    let requests = Amount { cpu: 500_000_000, memory: 256 * MI };
    Input { cluster: ClusterId::new("ams"), pods, objects, metrics: metrics(bed, requests) }
}

fn with<K: Describe>(bed: &TestTasks, objects: Objects, items: &[Arc<K>]) -> Objects {
    objects.with(bed.held(snapshot(items, StoreState::Live)))
}

fn counted(inputs: &[Input], namespaces: &[&str]) -> Vec<Counted> {
    let namespaces = namespaces.iter().map(|name| Arc::from(*name)).collect();
    let newest = AtomicU64::new(0);
    count(inputs, &namespaces, &Cancel::new(&newest, 0)).expect("no newer request")
}

#[test]
fn the_pods_count_by_phase_and_health_and_the_workloads_by_health_in_the_namespaces_of_the_filter() {
    let bed = TestTasks::new();
    let objects = with(&bed, Objects::default(), &fixture::<Deployment>());
    let input = input(&bed, objects);
    let all = counted(std::slice::from_ref(&input), &[]);
    let [ams] = all.as_slice() else { panic!("one cluster") };
    let phases = [("Running", 12), ("Pending", 6), ("Succeeded", 4), ("Failed", 3), ("Unknown", 0)];
    assert_eq!(ams.phases, phases);
    assert_eq!(ams.pods.total(), 25);
    assert_eq!((ams.nodes.is_some(), ams.warnings.is_some()), (false, false), "no store of the nodes or Events");
    let [(Kind::Deployment, deployments)] = ams.kinds.as_slice() else { panic!("the Deployments: {:?}", ams.kinds) };
    assert_eq!((deployments.total(), deployments.problems()), (4, 1), "the Deployment broken has no ready pod");

    let scene = counted(std::slice::from_ref(&input), &["scene"]);
    let [scene] = scene.as_slice() else { panic!("one cluster") };
    assert_eq!(scene.phases.first(), Some(&("Running", 10)), "the two pods of kube-system do not count");
    assert_eq!(scene.kinds.first().map(|(_, tally)| tally.total()), Some(2));
    let tallied = scene.pods;
    let healths = [ObjectHealth::Healthy, ObjectHealth::Progressing, ObjectHealth::Warning, ObjectHealth::Failed];
    assert_eq!(healths.map(|health| tallied.of(health)).iter().sum::<u32>(), 23);
}

#[test]
fn the_nodes_give_their_readiness_their_allocatable_sum_and_the_requests_of_the_last_poll() {
    let bed = TestTasks::new();
    let node = Arc::clone(fixture::<Node>().first().expect("the node"));
    let cordoned = changed(&node, |value| {
        value["metadata"]["name"] = "cordoned".into();
        value["spec"]["unschedulable"] = true.into();
        value["status"]["conditions"] = json!([{"type": "Ready", "status": "False"}]);
    });
    let two = input(&bed, with(&bed, Objects::default(), &[Arc::clone(&node), cordoned]));
    let counted = counted(&[two], &["scene"]);
    let nodes = counted.first().and_then(|ams| ams.nodes).expect("the nodes");
    assert_eq!((nodes.health.total(), nodes.ready, nodes.cordoned, nodes.health.problems()), (2, 1, 1, 1));
    let one = Resource::BOTH.map(|resource| resource.read(node.status.as_ref().and_then(|s| s.allocatable.as_ref())));
    assert_eq!(nodes.allocatable, one.map(|value| value.map(|value| value * 2)), "the sum of the two nodes");
    assert_eq!(nodes.requests, Amount { cpu: 500_000_000, memory: 256 * MI }, "the load of the node of the scene");

    // A node with no allocatable memory: no sum of memory, which would give a false part.
    let bare = changed(&node, |value| value["status"]["allocatable"] = json!({"cpu": "2"}));
    let input = input(&bed, with(&bed, Objects::default(), &[Arc::clone(&node), bare]));
    let nodes = super::count(&[input], &BTreeSet::new(), &Cancel::new(&AtomicU64::new(0), 0));
    let allocatable = nodes.and_then(|counted| counted.first()?.nodes).map(|nodes| nodes.allocatable);
    assert!(matches!(allocatable, Some([Some(_), None])), "{allocatable:?}");
}

#[test]
fn the_warnings_count_in_the_filter_and_the_newest_come_first() {
    let bed = TestTasks::new();
    let events = fixture::<Event>();
    let input = input(&bed, with(&bed, Objects::default(), &events));
    let all = counted(std::slice::from_ref(&input), &[]);
    let warnings = all.first().and_then(|ams| ams.warnings.clone()).expect("the warnings");
    assert_eq!((warnings.count, warnings.newest.len()), (31, NEWEST));
    let mut expected: Vec<&Arc<Event>> =
        events.iter().filter(|event| event.type_.as_deref() == Some("Warning")).collect();
    expected.sort_by_key(|event| std::cmp::Reverse(last_seen(event)));
    let times = |events: &[&Arc<Event>]| events.iter().map(|event| last_seen(event)).collect::<Vec<_>>();
    assert_eq!(times(&warnings.newest.iter().collect::<Vec<_>>()), times(&expected[..NEWEST]));

    let scene = counted(&[input], &["scene", "default"]);
    assert_eq!(scene.first().and_then(|ams| ams.warnings.as_ref()).map(|warnings| warnings.count), Some(29));
}

#[test]
fn the_same_snapshots_need_no_new_count_and_a_newer_request_stops_one() {
    let bed = TestTasks::new();
    let objects = with(&bed, Objects::default(), &fixture::<Deployment>());
    let input = input(&bed, objects.clone());
    assert!(input.is_same(&input.clone()));
    let other_pods = Input { pods: bed.held(snapshot(&fixture_pods(), StoreState::Live)), ..input.clone() };
    assert!(!input.is_same(&other_pods), "a new snapshot of the pods");
    let other_kind = Input { objects: with(&bed, objects.clone(), &fixture::<Node>()), ..input.clone() };
    assert!(!input.is_same(&other_kind), "one more kind");
    let new_deployments = Input { objects: with(&bed, objects, &fixture::<Deployment>()), ..input.clone() };
    assert!(!input.is_same(&new_deployments), "a new snapshot of a kind");
    let poll = Input { metrics: metrics(&bed, Amount::default()), ..input.clone() };
    assert!(!input.is_same(&poll), "a new poll of the metrics");

    let newest = AtomicU64::new(1);
    assert!(count(&[input], &BTreeSet::new(), &Cancel::new(&newest, 0)).is_none(), "a newer query waits");
}

fn changed(node: &Node, change: impl FnOnce(&mut serde_json::Value)) -> Arc<Node> {
    let mut value = serde_json::to_value(node).expect("JSON");
    change(&mut value);
    Arc::new(serde_json::from_value(value).expect("a node"))
}
