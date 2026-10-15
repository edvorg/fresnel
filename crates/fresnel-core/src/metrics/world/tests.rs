use std::time::SystemTime;

use fresnel_net::task::{TaskReport, Tasks};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use tokio::runtime::Handle;

use super::*;
use crate::fixture;
use crate::index::IndexState;
use crate::kinds::Meta;
use crate::published::{Bin, Disposal, Notice, Publisher};
use crate::store::StoreState;

fn snapshot<T: Object>(objects: Vec<T>, listed: bool) -> Snapshot<T> {
    let items = objects.into_iter().map(Arc::new).collect();
    Snapshot {
        generation: 1,
        state: StoreState::Live,
        since: SystemTime::UNIX_EPOCH,
        last_contact: None,
        listed,
        dropped: 0,
        items,
    }
}

fn reader<T: Object>(snapshot: Snapshot<T>, bin: &Bin) -> Reader<Snapshot<T>> {
    Publisher::new(snapshot, bin.clone(), Notice::new()).reader()
}

fn workload<K: Describe>(bin: &Bin) -> Box<dyn Workloads> {
    Box::new(Store { kind: K::ID, reader: reader(snapshot(fixture::all::<K>(), true), bin) })
}

/// The inputs of the scene of the k3s fixture. The store of the nodes has no complete list.
pub(in crate::metrics) fn scene(bin: &Bin) -> Inputs {
    let pods = snapshot(fixture::all::<Pod>(), true);
    let (replica_sets, jobs) = (snapshot(fixture::all(), true), snapshot(fixture::all(), true));
    let slices = snapshot(fixture::all::<EndpointSlice>(), true);
    let owners = Index::<Owners>::of_snapshots(&replica_sets, &jobs, &pods, &slices, IndexState::Following);
    let controllers: Box<dyn Workloads> =
        Box::new(Store { kind: Kind::ReplicationController, reader: reader(snapshot(Vec::<Meta>::new(), true), bin) });
    Inputs {
        pods: reader(pods, bin),
        nodes: reader(snapshot(Vec::new(), false), bin),
        namespaces: reader(snapshot(fixture::all(), true), bin),
        owners: Publisher::new(owners, bin.clone(), Notice::new()).reader(),
        workloads: vec![
            workload::<Deployment>(bin),
            workload::<StatefulSet>(bin),
            workload::<DaemonSet>(bin),
            workload::<ReplicaSet>(bin),
            controllers,
            workload::<Job>(bin),
            workload::<CronJob>(bin),
        ],
    }
}

fn key(name: &str) -> ObjectKey {
    ObjectKey::new(fixture::SCENE, name)
}

fn pods_of(world: &World, kind: Kind, name: &str) -> Vec<String> {
    let pods = world.workloads.get(&kind).and_then(|known| known.items.get(&key(name)));
    pods.into_iter().flatten().map(|pod| pod.name.to_string()).collect()
}

#[tokio::test]
async fn a_poll_reads_the_pods_of_each_workload_from_the_owner_index() {
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task ended: {report}"));
    let disposal = Disposal::start(&tasks);
    let world = scene(&disposal.bin()).read();
    assert_eq!(pods_of(&world, Kind::Deployment, "web"), ["web-6888b778b6-75md7", "web-6888b778b6-p2twl"]);
    assert_eq!(pods_of(&world, Kind::ReplicaSet, "web-64cb9bd5d4"), Vec::<String>::new(), "an old ReplicaSet");
    assert_eq!(pods_of(&world, Kind::StatefulSet, "db"), ["db-0", "db-1"]);
    assert_eq!(pods_of(&world, Kind::DaemonSet, "agent"), ["agent-8djvd"]);
    assert_eq!(pods_of(&world, Kind::CronJob, "tick"), ["tick-manual-d6v5w"]);
    assert_eq!(pods_of(&world, Kind::Job, "once"), ["once-r7w6j"]);
    // Two Jobs that own each other: the walk ends, with no pod of a loop counted twice.
    assert_eq!(pods_of(&world, Kind::Job, "loop-a"), ["loop-a-zrwmv"]);
    assert_eq!(world.workloads.len(), 7);

    assert!(world.runs(&key("web-6888b778b6-75md7")) && world.runs(&key("init-wait")));
    assert!(!world.runs(&key("done")) && !world.runs(&key("failed")) && !world.runs(&key("no-such-pod")));
}

/// The rule of `kubectl describe node`: a pod that succeeded or failed holds nothing.
#[tokio::test]
async fn the_load_of_a_node_counts_each_pod_on_it_that_is_not_done() {
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task ended: {report}"));
    let disposal = Disposal::start(&tasks);
    let world = scene(&disposal.bin()).read();
    // 16 pods have the node and are not done. Only coredns has requests: 100m and 70 MiB.
    let load = NodeLoad { requests: Amount { cpu: 100_000_000, memory: 70 * 1024 * 1024 }, pods: 16 };
    assert_eq!(world.loads.into_iter().collect::<Vec<_>>(), [(Arc::from("35f62a67b541"), load)]);
}

#[tokio::test]
async fn an_object_exists_when_its_store_has_it_and_is_not_known_before_a_complete_list() {
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task ended: {report}"));
    let disposal = Disposal::start(&tasks);
    let world = scene(&disposal.bin()).read();
    let exists = |key: SeriesKey| world.exists(&key);
    assert_eq!(exists(SeriesKey::Pod(key("done"))), Some(true));
    assert_eq!(exists(SeriesKey::Container { pod: key("gone"), name: "app".into() }), Some(false));
    assert_eq!(exists(SeriesKey::Workload(Kind::Deployment, key("web"))), Some(true));
    assert_eq!(exists(SeriesKey::Workload(Kind::StatefulSet, key("web"))), Some(false));
    assert_eq!(exists(SeriesKey::Workload(Kind::Service, key("web"))), None, "no store of workloads");
    assert_eq!(exists(SeriesKey::Namespace("scene-ending".into())), Some(true));
    assert_eq!(exists(SeriesKey::Namespace("gone".into())), Some(false));
    assert_eq!(exists(SeriesKey::Node("35f62a67b541".into())), None, "the store of the nodes has no list");
    assert_eq!(exists(SeriesKey::Cluster), Some(true));
}
