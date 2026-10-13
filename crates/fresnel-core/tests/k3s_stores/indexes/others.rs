//! The reference index and the Event index of the app against the API and kubectl, and
//! the owner index after a change of the cluster.

use fresnel_core::index::{Basis, ClusterIndexes, read_events};
use fresnel_core::kinds::Kind;
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::core::v1::{Node, Pod};
use tokio::time::timeout;

use super::read::{columns, objects, text, until_equal};
use super::scene::NAMESPACE;
use crate::bed::{App, Cluster, LIMIT};

const NAME: &str = "NAME:.metadata.name";

/// The names of some objects of the scene by a query of kubectl. The API server applies
/// the selector, so the query uses no rule of the app.
async fn names(cluster: &Cluster, what: &str, selector: &[&str]) -> Vec<String> {
    let mut names = columns(&cluster.k3s, what, &[&["--namespace", NAMESPACE], selector].concat(), NAME).await.concat();
    names.sort_unstable();
    names
}

/// "Used by": the pods of an object in the index against a selection of the server.
pub(crate) async fn each_object_has_the_pods_that_use_it(cluster: &Cluster, app: &App, indexes: &ClusterIndexes) {
    let index = indexes.references().read();
    let used_by = |kind: Kind, namespace: &str, name: &str| {
        let answer = index.used_by(kind, &ObjectKey::new(namespace, name));
        assert_eq!(answer.basis, Basis::Current);
        let mut pods: Vec<(String, String)> =
            answer.found.into_iter().map(|pod| (pod.namespace.to_string(), pod.name.to_string())).collect();
        pods.sort_unstable();
        pods
    };
    let in_scene = |names: Vec<String>| names.into_iter().map(|name| (NAMESPACE.to_owned(), name)).collect::<Vec<_>>();

    // The pods of the Deployment `web` use these objects, and no other pod does.
    let web = in_scene(names(cluster, "pods", &["--selector", "app=web"]).await);
    assert_eq!(web.len(), 2);
    for (kind, name) in [(Kind::ConfigMap, "web-config"), (Kind::Secret, "web-secret"), (Kind::Secret, "registry-key")]
    {
        assert_eq!(used_by(kind, NAMESPACE, name), web, "{kind:?} {name}");
    }
    for account in ["web", "default"] {
        let selector = format!("spec.serviceAccountName={account}");
        let pods = in_scene(names(cluster, "pods", &["--field-selector", &selector]).await);
        assert_eq!(used_by(Kind::ServiceAccount, NAMESPACE, account), pods, "the pods of the account {account}");
    }
    // The pods on the node, of each namespace.
    let nodes = app.stores.stores.full::<Node>().expect("the store of the nodes").read();
    let node = nodes.items.iter().next().expect("the node of k3s").0.name.to_string();
    let fields = "NAMESPACE:.metadata.namespace,NAME:.metadata.name";
    let selector = format!("spec.nodeName={node}");
    let on_node = columns(&cluster.k3s, "pods", &["--all-namespaces", "--field-selector", &selector], fields).await;
    let mut on_node: Vec<(String, String)> =
        on_node.into_iter().map(|words| (words[0].clone(), words[1].clone())).collect();
    on_node.sort_unstable();
    assert_eq!(used_by(Kind::Node, "", &node), on_node);
    println!(
        "the node has {} pods, and the account `default` of the scene has {}",
        on_node.len(),
        used_by(Kind::ServiceAccount, NAMESPACE, "default").len()
    );
}

/// The Events of an object in the index against a selection of the server, by the API
/// and by kubectl.
pub(crate) async fn each_object_has_its_events(cluster: &Cluster, app: &App, indexes: &ClusterIndexes) {
    let index = indexes.events().expect("the Events have a store");
    let pods = app.stores.stores.full::<Pod>().expect("the store of the pods").read();
    let crash = ObjectKey::new(NAMESPACE, "crash");
    let uid = pods.items.get(&crash).and_then(|pod| pod.metadata.uid.clone()).expect("the pod `crash`");
    // The pod starts again and again, so its Events change during the test.
    let of_pod = until_equal("the Events of the pod `crash`", async || {
        let answer = index.read().events_of(Kind::Pod, &crash, &uid);
        assert_eq!(answer.basis, Basis::Current);
        let mut of_index: Vec<String> = answer.found.iter().filter_map(|event| event.metadata.name.clone()).collect();
        of_index.sort_unstable();
        let selector = format!("involvedObject.uid={uid}");
        let events = objects(&cluster.api.gateway, &Kind::Event.info().path_in(NAMESPACE), Some(&selector)).await;
        let mut of_api: Vec<String> = events.iter().map(|event| text(event, "/metadata/name").to_owned()).collect();
        of_api.sort_unstable();
        let of_kubectl = names(cluster, "events", &["--field-selector", "involvedObject.name=crash"]).await;
        assert_eq!(of_api, of_kubectl, "the API and kubectl select the same Events");
        (of_index, of_api)
    })
    .await;
    // A cluster with no store of the Events reads them for one object: the same Events.
    let read = until_equal("the Events of the pod by a read", async || {
        let read =
            read_events(&cluster.api.gateway, Kind::Pod, &crash, &uid, 500).await.expect("the Events of the pod");
        assert!(!read.more);
        let of_read: Vec<Option<String>> = read.events.into_iter().map(|event| event.metadata.name).collect();
        let of_index = index.read().events_of(Kind::Pod, &crash, &uid).found;
        (of_read, of_index.iter().map(|event| event.metadata.name.clone()).collect())
    });
    assert_eq!(read.await.len(), of_pod.len(), "in the same sequence: the newest first");
    assert!(of_pod.len() >= 5, "a pod that starts again and again has many Events: {of_pod:?}");

    // The kubelet names its node with no UID of the API. The index finds these Events too.
    let nodes = app.stores.stores.full::<Node>().expect("the store of the nodes").read();
    let (key, node) = nodes.items.iter().next().expect("the node of k3s");
    let node_uid = node.metadata.uid.clone().expect("a UID");
    let of_node = until_equal("the Events of the node", async || {
        let answer = index.read().events_of(Kind::Node, key, &node_uid);
        let mut of_index: Vec<String> = answer.found.iter().filter_map(|event| event.metadata.name.clone()).collect();
        of_index.sort_unstable();
        let scope = ["--all-namespaces", "--field-selector", "involvedObject.kind=Node"];
        let mut of_kubectl = columns(&cluster.k3s, "events", &scope, NAME).await.concat();
        of_kubectl.sort_unstable();
        let read =
            read_events(&cluster.api.gateway, Kind::Node, key, &node_uid, 500).await.expect("the Events of the node");
        let mut of_read: Vec<String> = read.events.into_iter().filter_map(|event| event.metadata.name).collect();
        of_read.sort_unstable();
        assert_eq!(of_read, of_kubectl, "the read of the Events of one object finds them with no UID too");
        (of_index, of_kubectl)
    })
    .await;
    println!("the pod `crash` has {} Events and the node has {}", of_pod.len(), of_node.len());
    assert!(of_node.len() >= 5, "{of_node:?}");
}

/// One more pod for the Deployment `web`: the index follows the cluster.
pub(crate) async fn a_new_pod_of_a_deployment_reaches_the_index(cluster: &Cluster, indexes: &ClusterIndexes) {
    let deployment =
        objects(&cluster.api.gateway, &Kind::Deployment.info().path_in(NAMESPACE), Some("metadata.name=web")).await;
    let uid = text(&deployment[0], "/metadata/uid").to_owned();
    let owners = indexes.owners();
    let before = owners.read().generation;
    let scale = ["--namespace", NAMESPACE, "scale", "deployment/web", "--replicas=3"];
    cluster.k3s.kubectl(&scale).await.expect("kubectl scales the Deployment");

    let mut changes = indexes.changes();
    let three = async {
        loop {
            let answer = owners.read().pods_of(Kind::Deployment, &uid);
            if answer.found.len() == 3 {
                return answer;
            }
            changes.next().await;
        }
    };
    let answer = timeout(LIMIT, three).await.expect("the new pod is below the Deployment in time");
    assert_eq!(answer.basis, Basis::Current);
    let mut of_index: Vec<String> = answer.found.iter().map(|pod| pod.key.name.to_string()).collect();
    of_index.sort_unstable();
    assert_eq!(of_index, names(cluster, "pods", &["--selector", "app=web"]).await);
    assert!(owners.read().generation > before, "a new relation is a new generation");
}
