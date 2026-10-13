//! The owner index of the app against direct queries of the API and against kubectl.

use std::collections::{BTreeMap, BTreeSet};

use fresnel_core::index::{Basis, ClusterIndexes, Index, Owners};
use fresnel_core::kinds::Kind;
use fresnel_core::store::ObjectKey;
use serde_json::Value;

use super::read::{columns, objects, text, until_equal};
use super::scene::NAMESPACE;
use crate::bed::Cluster;

/// The kinds with an owner that the index reads, and the kinds of the owners of the scene.
const OWNED: [Kind; 3] = [Kind::ReplicaSet, Kind::Job, Kind::Pod];
const OWNERS: [Kind; 6] =
    [Kind::Deployment, Kind::StatefulSet, Kind::DaemonSet, Kind::CronJob, Kind::Job, Kind::ReplicaSet];
/// The same kinds, as `kubectl get` names them.
const OWNED_NAMES: &str = "replicasets,jobs,pods";
const OWNER_NAMES: &str = "deployments,statefulsets,daemonsets,cronjobs,jobs,replicasets";
const EVERYWHERE: &[&str] = &["--all-namespaces"];

/// One object of the cluster as a source other than the index tells it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Seen {
    kind: Kind,
    key: ObjectKey,
    uid: String,
    /// The UIDs in its `ownerReferences`.
    owners: Vec<String>,
}

/// The objects of these kinds by lists of the API.
async fn from_api(cluster: &Cluster, kinds: &[Kind]) -> Vec<Seen> {
    let mut seen = Vec::new();
    for kind in kinds {
        for object in objects(&cluster.api.gateway, &kind.info().path(), None).await {
            let owners = object.pointer("/metadata/ownerReferences").and_then(Value::as_array);
            let owners = owners.into_iter().flatten().map(|owner| text(owner, "/uid").to_owned()).collect();
            let key = ObjectKey::new(text(&object, "/metadata/namespace"), text(&object, "/metadata/name"));
            seen.push(Seen { kind: *kind, key, uid: text(&object, "/metadata/uid").to_owned(), owners });
        }
    }
    seen.sort();
    seen
}

/// The objects of these kinds by the output of kubectl.
async fn from_kubectl(cluster: &Cluster, kinds: &str) -> Vec<Seen> {
    let fields = "KIND:.kind,NAMESPACE:.metadata.namespace,NAME:.metadata.name,UID:.metadata.uid,\
                  OWNERS:.metadata.ownerReferences[*].uid";
    let line = |words: Vec<String>| {
        let [kind, namespace, name, uid, owners] = &words[..] else { panic!("five columns: {words:?}") };
        let owners = if owners == "<none>" { Vec::new() } else { owners.split(',').map(str::to_owned).collect() };
        let kind = Kind::named(kind).expect("a kind of the app");
        Seen { kind, key: ObjectKey::new(namespace, name), uid: uid.clone(), owners }
    };
    let mut seen: Vec<Seen> = columns(&cluster.k3s, kinds, EVERYWHERE, fields).await.into_iter().map(line).collect();
    seen.sort();
    seen
}

/// The pods below an owner, by a search through each object that has an owner.
fn pods_below(owner: &str, owned: &[Seen]) -> Vec<ObjectKey> {
    let (mut pods, mut visited, mut open) = (BTreeSet::new(), BTreeSet::new(), vec![owner]);
    while let Some(owner) = open.pop() {
        if !visited.insert(owner) {
            continue;
        }
        for object in owned.iter().filter(|object| object.owners.iter().any(|uid| uid == owner)) {
            if object.kind == Kind::Pod {
                pods.insert(object.key.clone());
            } else {
                open.push(&object.uid);
            }
        }
    }
    pods.into_iter().collect()
}

/// The pods below each owner of the cluster, by its kind and name.
type Below = BTreeMap<(Kind, ObjectKey), Vec<ObjectKey>>;

fn by_search(owners: &[Seen], owned: &[Seen]) -> Below {
    owners.iter().map(|owner| ((owner.kind, owner.key.clone()), pods_below(&owner.uid, owned))).collect()
}

fn by_index(index: &Index<Owners>, owners: &[Seen]) -> Below {
    let pods = |owner: &Seen| {
        let answer = index.pods_of(owner.kind, &owner.uid);
        assert_eq!(answer.basis, Basis::Current, "each store is live");
        answer.found.into_iter().map(|pod| pod.key).collect()
    };
    owners.iter().map(|owner| ((owner.kind, owner.key.clone()), pods(owner))).collect()
}

/// The pods below each workload: the index, a search through lists of the API, and a
/// search through the output of kubectl give the same.
pub(crate) async fn each_owner_has_the_pods_of_the_api_and_of_kubectl(cluster: &Cluster, indexes: &ClusterIndexes) {
    let index = indexes.owners();
    let below = until_equal("the pods below each owner", async || {
        let (owners, owned) = (from_api(cluster, &OWNERS).await, from_api(cluster, &OWNED).await);
        let of_kubectl = (from_kubectl(cluster, OWNER_NAMES).await, from_kubectl(cluster, OWNED_NAMES).await);
        assert_eq!((&owners, &owned), (&of_kubectl.0, &of_kubectl.1), "kubectl and the API show the same objects");
        (by_index(&index.read(), &owners), by_search(&owners, &owned))
    })
    .await;
    let pods = |kind: Kind, name: &str| below[&(kind, ObjectKey::new(NAMESPACE, name))].len();
    let with_pods = below.values().filter(|pods| !pods.is_empty()).count();
    println!("{} owners, {with_pods} of them with pods: the index is equal to the API and to kubectl", below.len());
    assert_eq!(
        (pods(Kind::Deployment, "web"), pods(Kind::StatefulSet, "db"), pods(Kind::DaemonSet, "agent")),
        (2, 2, 1)
    );
    assert_eq!((pods(Kind::CronJob, "tick"), pods(Kind::CronJob, "paused"), pods(Kind::Job, "fails")), (1, 0, 1));
    // The two Jobs that own each other: the walk ends, and each one has the one pod.
    assert_eq!((pods(Kind::Job, "loop-a"), pods(Kind::Job, "loop-b")), (1, 1));
    assert_eq!(below.len(), 18, "the owners of the scene and of k3s: {:?}", below.keys());
}

/// The pods behind each Service, with their state: the index against the EndpointSlices
/// that kubectl shows, and its ready pods against the Endpoints object of the API, which
/// a different controller of the cluster writes.
pub(crate) async fn each_service_has_the_pods_of_kubectl_and_of_the_api(cluster: &Cluster, indexes: &ClusterIndexes) {
    let index = indexes.owners();
    let scope = ["--namespace", NAMESPACE];
    let fields = "SERVICE:.metadata.labels.kubernetes\\.io/service-name,PODS:.endpoints[*].targetRef.name,\
                  READY:.endpoints[*].conditions.ready,ENDS:.endpoints[*].conditions.terminating";
    let services: Vec<String> = columns(&cluster.k3s, "services", &scope, "NAME:.metadata.name").await.concat();
    assert_eq!(services, ["db", "mixed", "nobody", "outside", "public", "web"]);
    let behind = until_equal("the pods behind each Service", async || {
        let mut of_kubectl: BTreeMap<String, BTreeMap<String, bool>> = BTreeMap::new();
        for words in columns(&cluster.k3s, "endpointslices", &scope, fields).await {
            let [service, pods, ready, ends] = &words[..] else { panic!("four columns: {words:?}") };
            let state = pods.split(',').zip(ready.split(',').zip(ends.split(',')));
            let pods = state
                .filter(|(pod, _)| *pod != "<none>")
                .map(|(pod, (ready, ends))| (pod.to_owned(), (ready, ends) == ("true", "false")));
            of_kubectl.entry(service.clone()).or_default().extend(pods);
        }
        let index = index.read();
        let mut of_index = BTreeMap::new();
        for service in &services {
            let answer = index.pods_of_service(&ObjectKey::new(NAMESPACE, service));
            assert_eq!(answer.basis, Basis::Current);
            let pods: BTreeMap<String, bool> =
                answer.found.into_iter().map(|pod| (pod.pod.name.to_string(), pod.ready)).collect();
            if !index.slices_of(&ObjectKey::new(NAMESPACE, service)).found.is_empty() {
                of_index.insert(service.clone(), pods);
            }
        }
        (of_index, of_kubectl)
    })
    .await;
    let ready = |service: &str| {
        behind[service].iter().filter(|(_, ready)| **ready).map(|(pod, _)| pod.clone()).collect::<Vec<_>>()
    };
    assert_eq!((behind["web"].len(), behind["db"].len(), behind["nobody"].len(), behind["mixed"].len()), (2, 2, 0, 3));
    assert_eq!(ready("mixed"), ["sidecar"], "of three pods, one is not ready and one ends");
    assert!(!behind.contains_key("outside"), "a Service with no slice");

    for service in ["web", "db", "mixed", "nobody"] {
        let endpoints = objects(
            &cluster.api.gateway,
            &Kind::Endpoints.info().path_in(NAMESPACE),
            Some(&format!("metadata.name={service}")),
        )
        .await;
        let addresses = endpoints.iter().flat_map(|endpoints| endpoints["subsets"].as_array().into_iter().flatten());
        let addresses = addresses.flat_map(|subset| subset["addresses"].as_array().into_iter().flatten());
        let mut of_api: Vec<String> = addresses.map(|address| text(address, "/targetRef/name").to_owned()).collect();
        of_api.sort_unstable();
        assert_eq!(ready(service), of_api, "the ready pods behind `{service}`");
    }
}
