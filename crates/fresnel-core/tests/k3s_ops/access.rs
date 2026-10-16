//! The permissions of a user with a Role in one namespace: the rules decide each action, an
//! access review gives the exact answer, and the real request of a forbidden action fails
//! with the message of the server.

use fresnel_core::cluster::Registry;
use fresnel_core::kinds::{Action, Kind};
use fresnel_core::net::NetSettings;
use fresnel_core::ops::access::{self, Attributes, Decision};
use fresnel_core::ops::{Cancel, JobState, Operation, Order, Parts, Propagation, Target};
use fresnel_core::store::ObjectKey;
use serde_json::json;
use tokio::runtime::Handle;

use crate::bed::{APP, Bed, run_parts};
use crate::common::{connect, registry, until_live};
use crate::user::{create, user};

const NAME: &str = "fresnel-viewer";

pub(crate) async fn a_forbidden_action_is_denied_with_its_reason_and_its_request_fails(bed: &Bed) {
    let b = &bed.b;
    let api = &b.link.gateway;
    let kubeconfig = user(api, &b.k3s, NAME, &json!({})).await;
    let rules = json!([
        {"apiGroups": ["apps"], "resources": ["deployments"], "verbs": ["get", "list", "watch"]},
        {"apiGroups": ["apps"], "resources": ["deployments/scale"], "resourceNames": ["web"], "verbs": ["patch"]},
    ]);
    let rbac = format!("/apis/rbac.authorization.k8s.io/v1/namespaces/{APP}");
    create(api, &format!("{rbac}/roles"), json!({"metadata": {"name": NAME}, "rules": rules})).await;
    let binding = json!({
        "metadata": {"name": NAME},
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "Role", "name": NAME},
        "subjects": [{"apiGroup": "rbac.authorization.k8s.io", "kind": "User", "name": NAME}],
    });
    create(api, &format!("{rbac}/rolebindings"), binding).await;

    let registry: Registry = registry(&bed.tasks, &Handle::current(), &kubeconfig, NetSettings::default());
    let viewer = connect(&registry, "default");
    until_live(&viewer).await;
    let gateway = &viewer.gateway;

    let rules = access::read_rules(gateway, APP).await.expect("the rules of the namespace");
    assert!(!rules.is_incomplete(), "RBAC lists each rule");
    let decide = |action, name: &str| {
        let key = ObjectKey::new(APP, name);
        Decision::all(access::needs(action, Kind::Deployment, &key, "").iter().map(|request| rules.decide(request)))
    };
    assert_eq!(decide(Action::Scale, "web"), Decision::Allowed);
    assert_eq!(decide(Action::ViewManifest, "paused"), Decision::Allowed);
    let denied = "no rule lets you patch deployments/scale (apps) paused in app";
    assert_eq!(decide(Action::Scale, "paused"), Decision::Denied(denied.into()), "the name is not in the rule");
    assert_eq!(
        decide(Action::Delete, "web"),
        Decision::Denied("no rule lets you delete deployments (apps) web in app".into())
    );
    assert!(matches!(decide(Action::Restart, "web"), Decision::Denied(_)));

    let attributes = |verb, resource, subresource, namespace: &str, name: &str| Attributes {
        verb,
        group: if resource == "nodes" { "" } else { "apps" },
        resource,
        subresource,
        namespace: namespace.into(),
        name: name.into(),
    };
    let scale = attributes("patch", "deployments", "scale", APP, "web");
    assert_eq!(access::check(gateway, &scale).await.expect("an answer"), Decision::Allowed);
    let delete = access::check(gateway, &attributes("delete", "deployments", "", APP, "web")).await;
    assert!(matches!(delete.expect("an answer"), Decision::Denied(_)));
    let node = access::check(gateway, &attributes("patch", "nodes", "", "", "any")).await;
    assert!(matches!(node.expect("an answer"), Decision::Denied(_)), "an action on the cluster");

    // The real request: the server refuses it with HTTP 403 and its message, and nothing changes.
    let target = Target { cluster: gateway.cluster().clone(), ..b.target(Kind::Deployment, "web").await };
    let order = Order::new(target, Operation::Delete(Propagation::Background)).expect("an order");
    let parts = Parts {
        gateway: gateway.clone(),
        deployments: None,
        stateful_sets: None,
        daemon_sets: None,
        jobs: None,
        pods: None,
        // A refused delete writes no file.
        downloads: std::env::temp_dir(),
        cancel: Cancel::pair().1,
        after: None,
    };
    let report = run_parts(&order, parts).await;
    let JobState::Failed(error) = &report.state else { panic!("a refusal: {report:#?}") };
    assert_eq!(error.status(), Some(403));
    assert!(error.to_string().contains("cannot delete resource \"deployments\""), "{error}");
    assert!(b.object(APP, Kind::Deployment, "web").await.is_some(), "the Deployment stays");
}
