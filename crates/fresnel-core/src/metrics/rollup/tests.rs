use std::collections::BTreeSet;

use super::*;
use crate::kinds::Kind;
use crate::metrics::resources::Amount;
use crate::metrics::world::Known;

const START: u32 = 1_792_058_400;

fn key(namespace: &str, name: &str) -> ObjectKey {
    ObjectKey::new(namespace, name)
}

fn amount(cpu: u64, memory: u64) -> Amount {
    Amount { cpu, memory }
}

fn pod(namespace: &str, name: &str, at: u32, containers: &[(&str, u64, u64)]) -> PodSample {
    let containers = containers.iter().map(|(name, cpu, memory)| (Arc::from(*name), amount(*cpu, *memory))).collect();
    PodSample { key: key(namespace, name), at, window: None, containers }
}

/// The pods of the store: `web-a`, `web-b` and `web-c` run, `web-d` is done, and `api` of an
/// other namespace runs. The Deployment `web` owns the four, `idle` owns `web-c` only.
fn world() -> World {
    let pods = [
        ("shop", "web-a", true),
        ("shop", "web-b", true),
        ("shop", "web-c", true),
        ("shop", "web-d", false),
        ("lab", "api", true),
    ];
    let owned = |names: &[&str]| names.iter().map(|name| key("shop", name)).collect::<Vec<_>>();
    let deployments =
        [(key("shop", "web"), owned(&["web-a", "web-b", "web-c", "web-d"])), (key("shop", "idle"), owned(&["web-c"]))];
    World {
        pods: Known {
            listed: true,
            items: pods.iter().map(|(namespace, name, runs)| (key(namespace, name), *runs)).collect(),
        },
        nodes: Known {
            listed: true,
            items: ["node-a", "node-b", "node-c"].into_iter().map(Arc::from).collect::<BTreeSet<_>>(),
        },
        workloads: [(Kind::Deployment, Known { listed: true, items: deployments.into_iter().collect() })].into(),
        ..World::default()
    }
}

#[test]
fn each_container_and_each_pod_has_the_value_and_the_time_of_its_item() {
    let pods = [pod("shop", "web-a", START + 5, &[("app", 10, 100), ("proxy", 1, 10)])];
    let samples = of_pods(&pods, &World::default());
    let one = |at, cpu, memory| Sample { at, cover: Cover::ONE, usage: amount(cpu, memory) };
    let container = |name: &str| SeriesKey::Container { pod: key("shop", "web-a"), name: name.into() };
    assert_eq!(
        samples,
        [
            (container("app"), one(START + 5, 10, 100)),
            (container("proxy"), one(START + 5, 1, 10)),
            (SeriesKey::Pod(key("shop", "web-a")), one(START + 5, 11, 110)),
            (SeriesKey::Namespace("shop".into()), one(START + 5, 11, 110)),
        ]
    );
}

/// Research 11 section 6: a missing sample is not zero, and the sum tells "2 of 3 pods".
#[test]
fn a_workload_is_the_sum_of_its_pods_with_a_value_and_expects_each_pod_that_runs() {
    let pods = [
        pod("shop", "web-a", START + 5, &[("app", 10, 100)]),
        pod("shop", "web-b", START + 9, &[("app", 20, 200)]),
        pod("lab", "api", START + 7, &[("app", 1, 1)]),
    ];
    let samples = of_pods(&pods, &world());
    let of = |key: SeriesKey| samples.iter().find(|(found, _)| *found == key).map(|(_, sample)| *sample);
    let web = Sample { at: START + 9, cover: Cover { sampled: 2, of: 3 }, usage: amount(30, 300) };
    assert_eq!(of(SeriesKey::Workload(Kind::Deployment, key("shop", "web"))), Some(web));
    assert_eq!(of(SeriesKey::Workload(Kind::Deployment, key("shop", "idle"))), None, "no pod has a value");
    assert_eq!(of(SeriesKey::Namespace("shop".into())), Some(web), "three pods of the namespace run");
    let lab = Sample { at: START + 7, cover: Cover::ONE, usage: amount(1, 1) };
    assert_eq!(of(SeriesKey::Namespace("lab".into())), Some(lab));
    assert_eq!(of(SeriesKey::Namespace("none".into())), None);
}

#[test]
fn the_cluster_is_the_sum_of_its_nodes_and_expects_each_node_of_its_store() {
    let node =
        |name: &str, at, cpu, memory| NodeSample { name: name.into(), at, window: None, usage: amount(cpu, memory) };
    let samples = of_nodes(&[node("node-a", START + 3, 100, 1_000), node("node-b", START + 4, 200, 2_000)], &world());
    let cluster = Sample { at: START + 4, cover: Cover { sampled: 2, of: 3 }, usage: amount(300, 3_000) };
    let one = Sample { at: START + 3, cover: Cover::ONE, usage: amount(100, 1_000) };
    assert_eq!(samples.first(), Some(&(SeriesKey::Node("node-a".into()), one)));
    assert_eq!(samples.last(), Some(&(SeriesKey::Cluster, cluster)));
    assert_eq!(samples.len(), 3);
    assert_eq!(of_nodes(&[], &world()), [], "no node, no value of the cluster");
}
