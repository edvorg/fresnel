use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::discovery::v1::EndpointSlice;

use super::*;
use crate::fixture::{self, SCENE};
use crate::index::bed::{Bed, TestStore, read};
use crate::index::task::Basis;
use crate::kinds::{Full, Object};
use crate::store::{StoreState, Wait};

/// The four stores that the owner index reads.
struct Stores {
    replica_sets: TestStore<ReplicaSet>,
    jobs: TestStore<Job>,
    pods: TestStore<Pod>,
    slices: TestStore<EndpointSlice>,
}

impl Stores {
    fn empty(bed: &Bed) -> Self {
        Self { replica_sets: bed.store(), jobs: bed.store(), pods: bed.store(), slices: bed.store() }
    }

    fn of_fixture(bed: &Bed) -> Self {
        Self { replica_sets: bed.fixture(), jobs: bed.fixture(), pods: bed.fixture(), slices: bed.fixture() }
    }

    fn indexer(&self) -> OwnerIndexer {
        OwnerIndexer::new(self.replica_sets.reader(), self.jobs.reader(), self.pods.reader(), self.slices.reader())
    }
}

fn uid(object: &impl Object) -> String {
    object.meta().uid.clone().expect("a UID")
}

fn names(found: &[ObjectRef]) -> Vec<String> {
    found.iter().map(|object| format!("{:?}/{}", object.kind, object.key.name)).collect()
}

/// The names of the objects of one kind in the fixture that start with `prefix`, with
/// the kind before each name.
fn fixture_names<K: Full>(prefix: &str) -> Vec<String> {
    let name = |object: K| object.meta().name.clone().expect("a name");
    let of_scene = |object: &K| object.meta().namespace.as_deref() == Some(SCENE);
    let all = fixture::all::<K>().into_iter().filter(of_scene).map(name);
    all.filter(|name| name.starts_with(prefix)).map(|name| format!("{:?}/{name}", K::KIND)).collect()
}

/// Each object of the fixture that the index reads, with the UIDs of its owners.
fn owned_objects() -> Vec<(ObjectRef, Vec<String>)> {
    fn of<K: Full>() -> Vec<(ObjectRef, Vec<String>)> {
        let entry = |object: K| {
            let meta = object.meta();
            let key =
                ObjectKey::new(meta.namespace.as_deref().expect("a namespace"), meta.name.as_deref().expect("a name"));
            let owners = meta.owner_references.iter().flatten().map(|owner| owner.uid.clone()).collect();
            (ObjectRef { kind: K::KIND, key, uid: uid(&object).into() }, owners)
        };
        fixture::all::<K>().into_iter().map(entry).collect()
    }
    [of::<ReplicaSet>(), of::<Job>(), of::<Pod>()].concat()
}

/// The pods below an owner, by a plain search through each object of the fixture.
fn pods_by_search(owner: &str, objects: &[(ObjectRef, Vec<String>)]) -> Vec<ObjectRef> {
    let (mut pods, mut visited, mut open) = (BTreeSet::new(), BTreeSet::new(), vec![owner.to_owned()]);
    while let Some(owner) = open.pop() {
        if !visited.insert(owner.clone()) {
            continue;
        }
        for (object, _) in objects.iter().filter(|(_, owners)| owners.contains(&owner)) {
            if object.kind == Kind::Pod {
                pods.insert(object.clone());
            } else {
                open.push(object.uid.to_string());
            }
        }
    }
    pods.into_iter().collect()
}

#[tokio::test]
async fn each_workload_of_the_fixture_finds_its_objects_and_its_pods() {
    let bed = Bed::new();
    let (index, changed) = read(&mut Stores::of_fixture(&bed).indexer());
    assert!(changed);

    // Deployment to ReplicaSet to Pod. The Deployment has the ReplicaSets of two rounds.
    let web = uid(&fixture::one::<Deployment>("web"));
    let owned = index.owned(Kind::Deployment, &web);
    assert_eq!((names(&owned.found), &owned.basis), (fixture_names::<ReplicaSet>("web-"), &Basis::Current));
    assert_eq!(owned.found.len(), 2);
    let pods = index.pods_of(Kind::Deployment, &web);
    assert_eq!((names(&pods.found), &pods.basis), (fixture_names::<Pod>("web-"), &Basis::Current));
    assert_eq!(pods.found.len(), 2);
    // The ReplicaSet of the first round has no pod, and that is a fact: each store is live.
    let rounds: Vec<Vec<String>> =
        owned.found.iter().map(|round| names(&index.pods_of(Kind::ReplicaSet, &round.uid).found)).collect();
    assert!(rounds.contains(&Vec::new()) && rounds.contains(&names(&pods.found)), "{rounds:?}");

    // StatefulSet and DaemonSet to Pod.
    let db = index.pods_of(Kind::StatefulSet, &uid(&fixture::one::<StatefulSet>("db")));
    assert_eq!(names(&db.found), ["Pod/db-0", "Pod/db-1"]);
    let agent = index.pods_of(Kind::DaemonSet, &uid(&fixture::one::<DaemonSet>("agent")));
    assert_eq!(names(&agent.found), fixture_names::<Pod>("agent-"));

    // CronJob to Job to Pod.
    let tick = uid(&fixture::one::<CronJob>("tick"));
    assert_eq!(names(&index.owned(Kind::CronJob, &tick).found), ["Job/tick-manual"]);
    assert_eq!(names(&index.pods_of(Kind::CronJob, &tick).found), fixture_names::<Pod>("tick-manual-"));
    assert_eq!(index.pods_of(Kind::CronJob, &uid(&fixture::one::<CronJob>("paused"))).found, [], "it never ran");

    // Each owner of the fixture: the index is equal to a search through each object.
    let objects = owned_objects();
    let owners: BTreeSet<&String> = objects.iter().flat_map(|(_, owners)| owners).collect();
    assert!(owners.len() >= 14, "{} owners", owners.len());
    for owner in owners {
        assert_eq!(index.pods_of(Kind::Deployment, owner).found, pods_by_search(owner, &objects), "{owner}");
    }
}

/// The review of the prototype found a walk with no end for two Jobs that own each other.
/// The API server of k3s takes such Jobs, and the fixture has them.
#[tokio::test]
async fn two_jobs_that_own_each_other_end_the_walk() {
    let bed = Bed::new();
    let (index, _) = read(&mut Stores::of_fixture(&bed).indexer());
    let (a, b) = (fixture::one::<Job>("loop-a"), fixture::one::<Job>("loop-b"));
    let owner_of = |job: &Job| job.metadata.owner_references.as_ref().expect("an owner")[0].uid.clone();
    assert_eq!((owner_of(&a), owner_of(&b)), (uid(&b), uid(&a)), "the loop of the scene");

    let pod = fixture_names::<Pod>("loop-a-");
    assert_eq!(pod.len(), 1);
    // In the sequence of the kinds: the pod of the Job, then the Job that names it as its owner.
    assert_eq!(names(&index.owned(Kind::Job, &uid(&a)).found), [pod.clone(), vec!["Job/loop-b".to_owned()]].concat());
    // The walk runs on a thread of its own: a walk with no end must fail the test, not hold it.
    let (sender, walks) = std::sync::mpsc::channel();
    let walk = move || {
        let below = |job: &Job| names(&index.pods_of(Kind::Job, &uid(job)).found);
        sender.send((below(&a), below(&b)))
    };
    #[expect(clippy::disallowed_methods, reason = "the test does not wait for a thread that can run with no end")]
    std::thread::spawn(walk);
    let (below_a, below_b) = walks.recv_timeout(Duration::from_secs(10)).expect("the walk ends");
    assert_eq!((below_a, below_b), (pod.clone(), pod), "below `loop-b` through `loop-a`");
}

/// Plan section 9.8: the index tells "no pods" from "pods not loaded".
#[tokio::test]
async fn an_answer_with_no_pod_tells_if_the_cluster_has_none_or_if_the_pods_are_not_loaded() {
    let bed = Bed::new();
    let mut stores = Stores::empty(&bed);
    let mut indexer = stores.indexer();
    let not_loaded = |index: &Index<Owners>, owner: Kind| match index.pods_of(owner, "uid-1") {
        Answer { found, basis: Basis::NotLoaded(source) } if found.is_empty() => Some(source.kind),
        Answer { basis: Basis::Current, .. } => None,
        other => panic!("{other:?}"),
    };

    let (index, changed) = read(&mut indexer);
    assert!(!changed);
    assert_eq!(not_loaded(&index, Kind::Deployment), Some(Kind::ReplicaSet), "the first store below the owner");
    assert_eq!(not_loaded(&index, Kind::CronJob), Some(Kind::Job));
    assert_eq!(not_loaded(&index, Kind::StatefulSet), Some(Kind::Pod));
    assert!(!index.pods_of(Kind::Deployment, "uid-1").basis.is_loaded());

    // The lists arrive one by one. The pods of a Deployment need the ReplicaSets and the pods.
    stores.replica_sets.list(Vec::new());
    let (index, _) = read(&mut indexer);
    assert_eq!(index.owned(Kind::Deployment, "uid-1"), Answer { found: Vec::new(), basis: Basis::Current });
    assert_eq!(not_loaded(&index, Kind::Deployment), Some(Kind::Pod));
    stores.pods.list(Vec::new());
    let (index, changed) = read(&mut indexer);
    assert!(!changed, "a list with no object is no new relation");
    assert_eq!(not_loaded(&index, Kind::Deployment), None, "now the cluster has no pod");
    assert_eq!(not_loaded(&index, Kind::StatefulSet), None);
    assert_eq!(
        not_loaded(&index, Kind::CronJob),
        Some(Kind::Job),
        "a store that an answer does not need does not count"
    );

    // A store that is not live any more: the answer can be old, and it says so.
    stores.pods.set_state(StoreState::Waiting(Wait::Channel));
    let (index, _) = read(&mut indexer);
    let Basis::Old(source) = index.pods_of(Kind::Deployment, "uid-1").basis else { panic!("an old answer") };
    assert_eq!((source.kind, source.listed, &source.state), (Kind::Pod, true, &StoreState::Waiting(Wait::Channel)));
    assert_eq!(index.owned(Kind::Deployment, "uid-1").basis, Basis::Current, "the ReplicaSets are live");
}

fn pod(name: &str, uid: &str, version: &str, owner: Option<&str>) -> Pod {
    let owners = owner.map_or(String::new(), |owner| {
        format!(r#","ownerReferences":[{{"apiVersion":"apps/v1","kind":"ReplicaSet","name":"set","uid":"{owner}"}}]"#)
    });
    let text = format!(
        r#"{{"metadata":{{"name":"{name}","namespace":"shop","uid":"{uid}","resourceVersion":"{version}"{owners}}}}}"#
    );
    serde_json::from_str(&text).expect("a pod")
}

#[tokio::test]
async fn a_new_status_changes_no_relation_and_a_new_owner_moves_the_pod() {
    let bed = Bed::new();
    let mut stores = Stores::empty(&bed);
    let mut indexer = stores.indexer();
    stores.pods.list(vec![pod("a", "uid-a", "1", Some("set-1")), pod("free", "uid-free", "1", None)]);
    let pods = |index: &Index<Owners>, owner: &str| {
        let found = index.pods_of(Kind::ReplicaSet, owner).found;
        found.iter().map(|pod| format!("{} {}", pod.key.name, pod.uid)).collect::<Vec<_>>()
    };
    let (index, changed) = read(&mut indexer);
    assert!(changed);
    assert_eq!(pods(&index, "set-1"), ["a uid-a"]);

    // Most events of a pod are a new status.
    stores.pods.apply(pod("a", "uid-a", "2", Some("set-1")));
    stores.pods.apply(pod("free", "uid-free", "2", None));
    assert!(!read(&mut indexer).1, "the same owners are no change of the index");

    stores.pods.apply(pod("a", "uid-a", "3", Some("set-2")));
    let (index, changed) = read(&mut indexer);
    assert!(changed);
    assert_eq!((pods(&index, "set-1"), pods(&index, "set-2")), (Vec::new(), vec!["a uid-a".to_owned()]));

    // A new pod with the name of the old one, in one step of the store.
    stores.pods.apply(pod("a", "uid-new", "4", Some("set-2")));
    let (index, changed) = read(&mut indexer);
    assert!(changed);
    assert_eq!(pods(&index, "set-2"), ["a uid-new"]);

    stores.pods.delete(&pod("a", "uid-new", "4", Some("set-2")));
    let (index, changed) = read(&mut indexer);
    assert!(changed);
    assert_eq!((pods(&index, "set-2"), index.relations().children.keys()), (Vec::new(), 0), "no entry stays");
}

fn service(name: &str) -> ObjectKey {
    ObjectKey::new(SCENE, name)
}

#[tokio::test]
async fn a_service_of_the_fixture_finds_its_slices_and_its_pods_with_their_state() {
    let bed = Bed::new();
    let mut stores = Stores::of_fixture(&bed);
    let mut indexer = stores.indexer();
    let (index, _) = read(&mut indexer);
    let pods = |index: &Index<Owners>, name: &str| -> BTreeMap<String, bool> {
        let answer = index.pods_of_service(&service(name));
        assert_eq!(answer.basis, Basis::Current);
        answer.found.into_iter().map(|pod| (pod.pod.name.to_string(), pod.ready)).collect()
    };
    let ready = |names: Vec<String>| -> BTreeMap<String, bool> {
        names.into_iter().map(|name| (name.trim_start_matches("Pod/").to_owned(), true)).collect()
    };
    assert_eq!(pods(&index, "web"), ready(fixture_names::<Pod>("web-")));
    assert_eq!(pods(&index, "public"), pods(&index, "web"), "a second Service with the same pods");
    assert_eq!(pods(&index, "db"), ready(vec!["db-0".to_owned(), "db-1".to_owned()]));
    // One pod is ready, one is not ready and one ends.
    let mixed = [("leaving", false), ("not-ready", false), ("sidecar", true)];
    assert_eq!(pods(&index, "mixed"), mixed.map(|(name, ready)| (name.to_owned(), ready)).into());
    assert_eq!(pods(&index, "nobody"), BTreeMap::new(), "a Service with no pod has a slice with no endpoint");
    assert_eq!(index.slices_of(&service("nobody")).found.len(), 1);
    assert_eq!(index.slices_of(&service("outside")).found, [], "no slice for a name of an other system");

    // An endpoint that is not ready any more: each change of a slice is a change of the index.
    let slice_of = |name: &str| index.slices_of(&service(name)).found.remove(0);
    let web = fixture::changed::<EndpointSlice>(&slice_of("web").name, |slice| {
        slice["endpoints"][0]["conditions"]["ready"] = false.into();
        slice["metadata"]["resourceVersion"] = "999999".into();
    });
    stores.slices.apply(web.clone());
    let (changed_index, changed) = read(&mut indexer);
    assert!(changed);
    assert_eq!(pods(&changed_index, "web").values().filter(|ready| **ready).count(), 1);
    assert_eq!(pods(&changed_index, "public").values().filter(|ready| **ready).count(), 2, "its own slice says ready");
    assert_eq!(pods(&index, "web").values().filter(|ready| **ready).count(), 2, "an index that a reader holds");

    // A slice that leaves its Service, and a slice that is gone.
    let mut moved = web;
    moved
        .metadata
        .labels
        .as_mut()
        .expect("labels")
        .insert("kubernetes.io/service-name".to_owned(), "nobody".to_owned());
    moved.metadata.resource_version = Some("1000000".to_owned());
    stores.slices.apply(moved.clone());
    let (index, _) = read(&mut indexer);
    assert_eq!((pods(&index, "web").len(), pods(&index, "nobody").len()), (0, 2));
    assert_eq!(index.slices_of(&service("nobody")).found.len(), 2);
    stores.slices.delete(&moved);
    let (index, _) = read(&mut indexer);
    assert_eq!((pods(&index, "nobody").len(), index.slices_of(&service("web")).found.len()), (0, 0));
}

#[tokio::test]
async fn the_pods_of_a_service_are_not_loaded_before_the_list_of_the_slices() {
    let bed = Bed::new();
    let mut stores = Stores::empty(&bed);
    let mut indexer = stores.indexer();
    stores.pods.list(Vec::new());
    let (index, _) = read(&mut indexer);
    let answer = index.pods_of_service(&service("web"));
    assert!(matches!(&answer.basis, Basis::NotLoaded(source) if source.kind == Kind::EndpointSlice), "{answer:?}");
    assert!(matches!(index.slices_of(&service("web")).basis, Basis::NotLoaded(_)));
    stores.slices.list(Vec::new());
    assert_eq!(
        read(&mut indexer).0.pods_of_service(&service("web")),
        Answer { found: Vec::new(), basis: Basis::Current }
    );
}
