//! The permissions through `Command::ReadRules` and `Command::CheckAccess`, as a view asks
//! for them. The user of the context `viewer` may read the Deployments of `app` in cluster
//! b and do nothing else, so the views turn the scale of `web` off with its reason, while
//! the admin of b may scale the same object. A refusal of the server with HTTP 403 drops
//! the rules of its namespace.

use std::path::PathBuf;

use fresnel_core::app::{AccessView, CheckState, Command, CoreView, RulesState};
use fresnel_core::cluster::{ClusterId, Gateway};
use fresnel_core::kinds::{Action, Kind};
use fresnel_core::ops::access::{self, Decision};
use fresnel_core::ops::{JobState, Operation, Order, Target};
use fresnel_core::store::ObjectKey;
use fresnel_testkit::K3s;
use k8s_openapi::api::apps::v1::Deployment;
use serde_json::json;

use crate::bed::{Bed, data};
use crate::scene::{APP, field, kubectl};
use crate::user::{create, user};

const NAME: &str = "fresnel-viewer";
/// The two contexts of cluster b: its admin and the user of the test.
const CONTEXTS: [&str; 2] = ["b", "viewer"];

/// Makes the user with a Role that lets it read the Deployments of `app`. Gives its kubeconfig.
pub(crate) async fn viewer(api: &Gateway, b: &K3s) -> PathBuf {
    let kubeconfig = user(api, b, NAME, &json!({})).await;
    let rules = json!([{"apiGroups": ["apps"], "resources": ["deployments"], "verbs": ["get", "list", "watch"]}]);
    let rbac = format!("/apis/rbac.authorization.k8s.io/v1/namespaces/{APP}");
    create(api, &format!("{rbac}/roles"), json!({"metadata": {"name": NAME}, "rules": rules})).await;
    let binding = json!({
        "metadata": {"name": NAME},
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "Role", "name": NAME},
        "subjects": [{"apiGroup": "rbac.authorization.k8s.io", "kind": "User", "name": NAME}],
    });
    create(api, &format!("{rbac}/rolebindings"), binding).await;
    kubeconfig
}

/// The permissions of a cluster in the view.
fn of<'a>(view: &'a CoreView, cluster: &str) -> &'a AccessView {
    &data(view, cluster).unwrap_or_else(|| panic!("{cluster} is connected")).access
}

fn rules<'a>(view: &'a CoreView, cluster: &str) -> Option<&'a RulesState> {
    of(view, cluster).rules.get(APP)
}

pub(crate) async fn the_rules_and_the_reviews_of_each_context_decide_its_actions(bed: &Bed) {
    for cluster in CONTEXTS {
        bed.core.send(Command::ReadRules { cluster: ClusterId::new(cluster), namespace: APP.into() });
    }
    let read =
        |view: &CoreView| CONTEXTS.iter().all(|cluster| matches!(rules(view, cluster), Some(RulesState::Read(_))));
    let view = bed.until("the rules of app for each context of b", read).await;
    let web = ObjectKey::new(APP, "web");
    let decide = |cluster, action| of(&view, cluster).decide(&access::needs(action, Kind::Deployment, &web, ""));
    assert_eq!(decide("b", Action::Scale), Decision::Allowed);
    let off = "no rule lets you patch deployments/scale (apps) web in app";
    assert_eq!(decide("viewer", Action::Scale), Decision::Denied(off.into()), "the scale is off with its reason");
    assert_eq!(
        decide("viewer", Action::Delete),
        Decision::Denied("no rule lets you delete deployments (apps) web in app".into())
    );
    assert_eq!(decide("viewer", Action::ViewManifest), Decision::Allowed, "the Role lets the user read it");

    // A kind with no namespace: no rules of a namespace tell, so a view asks the server
    // about each request of the action.
    let node = kubectl(&bed.b, &["get", "nodes", "--output", "jsonpath={.items[0].metadata.name}"]).await;
    let key = ObjectKey::new("", &node);
    let cordon = access::needs(Action::Cordon, Kind::Node, &key, "");
    for cluster in CONTEXTS {
        for attributes in &cordon {
            bed.core.send(Command::CheckAccess { cluster: ClusterId::new(cluster), attributes: attributes.clone() });
        }
    }
    let answered = |view: &CoreView| {
        let answered = |cluster| {
            cordon.iter().all(|asked| matches!(of(view, cluster).checks.get(asked), Some(CheckState::Answered(_))))
        };
        CONTEXTS.iter().all(|cluster| answered(cluster))
    };
    let view = bed.until("the answers of the access reviews", answered).await;
    assert_eq!(of(&view, "b").decide(&cordon), Decision::Allowed);
    let Decision::Denied(reason) = of(&view, "viewer").decide(&cordon) else {
        panic!("the server denies the cordon of the viewer: {:#?}", of(&view, "viewer"))
    };
    let start = format!("the server does not let you get nodes {node} in the cluster");
    assert!(reason.starts_with(&start), "{reason}");

    // The real request of the viewer: HTTP 403, nothing changes, and the rules of app go.
    let replicas = field(&bed.b, "deployment/web", "{.spec.replicas}").await;
    let target = Target { cluster: ClusterId::new("viewer"), ..bed.target::<Deployment>("b", "web") };
    let job = bed.run_one(Order::new(target, Operation::Scale(5)).expect("an order")).await;
    let JobState::Failed(error) = &job.report.state else { panic!("a refusal: {job:#?}") };
    assert_eq!(error.status(), Some(403), "{error}");
    let view = bed.until("the refusal drops the rules of app", |view| rules(view, "viewer").is_none()).await;
    assert!(matches!(rules(&view, "b"), Some(RulesState::Read(_))), "the rules of the admin stay");
    assert_eq!(field(&bed.b, "deployment/web", "{.spec.replicas}").await, replicas, "nothing changed");
}
