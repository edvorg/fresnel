use k8s_openapi::api::core::v1::{Node, Pod};

use super::*;
use crate::fixture::{self, SCENE};
use crate::index::bed::{Bed, read};
use crate::index::task::Basis;
use crate::store::StoreState;

fn target(kind: Kind, name: &str) -> Target {
    Target { kind, key: ObjectKey::new(if kind == Kind::Node { "" } else { SCENE }, name) }
}

/// The names of the pods of the scene that fit, in their sequence.
fn pods_where(fits: impl Fn(&Pod) -> bool) -> Vec<String> {
    let of_scene = |pod: &Pod| pod.metadata.namespace.as_deref() == Some(SCENE);
    let all = fixture::all::<Pod>().into_iter().filter(|pod| of_scene(pod) && fits(pod));
    all.map(|pod| pod.metadata.name.expect("a name")).collect()
}

fn named(prefix: &str) -> Vec<String> {
    pods_where(|pod| pod.metadata.name.as_deref().is_some_and(|name| name.starts_with(prefix)))
}

#[test]
fn a_pod_of_the_fixture_names_its_config_maps_its_secrets_its_account_and_its_node() {
    let node = fixture::all::<Node>().remove(0).metadata.name.expect("a name");
    let expected = [
        target(Kind::Node, &node),
        target(Kind::ConfigMap, "kube-root-ca.crt"),
        target(Kind::ConfigMap, "web-config"),
        target(Kind::ConfigMap, "web-env"),
        target(Kind::Secret, "registry-key"),
        target(Kind::Secret, "web-secret"),
        target(Kind::Secret, "web-tls"),
        target(Kind::ServiceAccount, "web"),
    ];
    assert_eq!(references_of(&fixture::one::<Pod>("web-")), expected.into());

    // A pod with no node yet, and a reference in the environment of one value.
    let waits = references_of(&fixture::one::<Pod>("unschedulable"));
    assert_eq!(waits, [target(Kind::ConfigMap, "kube-root-ca.crt"), target(Kind::ServiceAccount, "default")].into());
    assert!(references_of(&fixture::one::<Pod>("no-config")).contains(&target(Kind::ConfigMap, "no-such-config")));
    assert_eq!(references_of(&Pod::default()), BTreeSet::new());

    // The other containers of a pod, and a Secret as a volume or as a set of variables.
    let mut pod = fixture::one::<Pod>("init-wait");
    let spec = pod.spec.as_mut().expect("a spec");
    fn from<T: serde::de::DeserializeOwned>(text: &str) -> T {
        serde_json::from_str(text).expect("JSON of the API")
    }
    spec.init_containers.as_mut().expect("init containers")[0].env_from =
        from(r#"[{"secretRef":{"name":"init-set"}}]"#);
    spec.ephemeral_containers =
        from(r#"[{"name":"debug","env":[{"name":"A","valueFrom":{"secretKeyRef":{"name":"debug-key","key":"a"}}}]}]"#);
    spec.volumes.as_mut().expect("volumes").push(from(r#"{"name":"keys","secret":{"secretName":"volume-keys"}}"#));
    let secrets: Vec<String> = references_of(&pod)
        .into_iter()
        .filter(|target| target.kind == Kind::Secret)
        .map(|target| target.key.name.to_string())
        .collect();
    assert_eq!(secrets, ["debug-key", "init-set", "volume-keys"]);
}

#[tokio::test]
async fn an_object_of_the_fixture_finds_the_pods_that_use_it() {
    let bed = Bed::new();
    let pods = bed.fixture::<Pod>();
    let (index, changed) = read(&mut ReferenceIndexer::new(pods.reader()));
    assert!(changed);
    let used_by = |kind: Kind, name: &str| {
        let answer = index.used_by(kind, &target(kind, name).key);
        assert_eq!(answer.basis, Basis::Current);
        answer.found.into_iter().map(|pod| pod.name.to_string()).collect::<Vec<_>>()
    };
    let web = named("web-");
    assert_eq!(web.len(), 2);
    for (kind, name) in [
        (Kind::ConfigMap, "web-config"),
        (Kind::Secret, "web-secret"),
        (Kind::ServiceAccount, "web"),
        // A reference needs no object: these three do not exist in the scene.
        (Kind::ConfigMap, "web-env"),
        (Kind::Secret, "web-tls"),
        (Kind::Secret, "registry-key"),
    ] {
        assert_eq!(used_by(kind, name), web, "{kind:?} {name}");
    }
    assert_eq!(used_by(Kind::ConfigMap, "no-such-config"), ["no-config"]);
    assert_eq!(used_by(Kind::ConfigMap, "no-such-object"), Vec::<String>::new());
    assert_eq!(
        used_by(Kind::ConfigMap, "kube-root-ca.crt"),
        pods_where(|_| true),
        "each pod has the CA of the cluster"
    );
    assert_eq!(used_by(Kind::ServiceAccount, "default").len() + web.len(), pods_where(|_| true).len());

    // The pods on the node, of each namespace.
    let node = fixture::all::<Node>().remove(0).metadata.name.expect("a name");
    let on_node = index.used_by(Kind::Node, &ObjectKey::new("", &node)).found;
    let scheduled = |pod: &Pod| pod.spec.as_ref().is_some_and(|spec| spec.node_name.as_deref() == Some(&node));
    assert_eq!(on_node.len(), fixture::all::<Pod>().iter().filter(|pod| scheduled(pod)).count());
    assert_eq!(on_node.iter().filter(|pod| &*pod.namespace == SCENE).count(), pods_where(scheduled).len());
    assert!(!on_node.iter().any(|pod| ["unschedulable", "gated"].contains(&&*pod.name)), "they have no node");
    assert_eq!(on_node.iter().filter(|pod| &*pod.namespace == "kube-system").count(), 2, "the pods of k3s");
}

#[tokio::test]
async fn a_new_status_changes_no_reference_and_a_pod_that_is_gone_leaves_no_entry() {
    let bed = Bed::new();
    let mut pods = bed.store::<Pod>();
    let mut indexer = ReferenceIndexer::new(pods.reader());
    let (index, changed) = read(&mut indexer);
    let config = target(Kind::ConfigMap, "web-config").key;
    assert!(!changed);
    assert!(
        matches!(index.used_by(Kind::ConfigMap, &config).basis, Basis::NotLoaded(source) if source.kind == Kind::Pod)
    );

    // A pod with no node, as the scheduler sees it first.
    let web = fixture::one::<Pod>("web-");
    let node = web.spec.as_ref().and_then(|spec| spec.node_name.clone()).expect("a node");
    let version = |pod: &mut Pod, version: &str| pod.metadata.resource_version = Some(version.to_owned());
    let mut pending = web.clone();
    pending.spec.as_mut().expect("a spec").node_name = None;
    version(&mut pending, "1");
    pods.list(vec![pending.clone()]);
    let (index, changed) = read(&mut indexer);
    let on_node = |index: &Index<References>| index.used_by(Kind::Node, &ObjectKey::new("", &node)).found.len();
    assert!(changed);
    assert_eq!((index.used_by(Kind::ConfigMap, &config).found.len(), on_node(&index)), (1, 0));

    version(&mut pending, "2");
    pods.apply(pending);
    assert!(!read(&mut indexer).1, "a new version with the same references");
    pods.apply(web.clone());
    let (index, changed) = read(&mut indexer);
    assert!(changed, "the pod has a node now");
    assert_eq!(on_node(&index), 1);

    pods.set_state(StoreState::Stopped);
    pods.delete(&web);
    let (index, changed) = read(&mut indexer);
    assert!(changed);
    assert_eq!((on_node(&index), index.relations().users.keys()), (0, 0));
    assert!(
        matches!(index.used_by(Kind::ConfigMap, &config).basis, Basis::Old(source) if source.state == StoreState::Stopped)
    );
}
