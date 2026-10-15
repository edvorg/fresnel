//! The views of the Access control kinds: the rules of a role, the role and the subjects of a
//! binding as links, and a ServiceAccount with the bindings that name it.

use std::collections::BTreeSet;
use std::sync::Arc;

use fresnel_core::app::{ClusterRow, Command};
use fresnel_core::kinds::Kind;
use fresnel_core::store::{Snapshot, StoreState};
use k8s_openapi::api::core::v1::ServiceAccount;
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding};
use serde_json::json;

use super::{and, follow, shows_part};
use crate::data::{changed_object, held, of_kinds, store};
use crate::fake::{shows, until};
use crate::kinds::scene::live;
use crate::kinds::{in_panel, missing, times_in_panel};
use crate::pods::scenes::forbidden;
use crate::workloads::scene::opened;

#[test]
fn a_role_shows_its_rules_as_kubectl_describe_prints_them() {
    let window = opened::<Role>(vec![live("ams-prod", &of_kinds::<Role>())], "reader");
    let texts = ["RULES", "Resources", "URLs", "Names", "Verbs", "pods", "get, list"];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());

    let roles = of_kinds::<ClusterRole>();
    let viewer = roles.iter().find(|role| role.metadata.name.as_deref() == Some("fresnel-viewer")).expect("a role");
    let aggregated = changed_object(&**viewer, |value| {
        value["aggregationRule"] = json!({"clusterRoleSelectors": [{"matchLabels": {"fresnel.test/view": "true"}}]});
        value["rules"].as_array_mut().expect("rules").push(json!({"nonResourceURLs": ["/healthz"], "verbs": ["get"]}));
    });
    let window = opened::<ClusterRole>(vec![live("ams-prod", &[aggregated])], "fresnel-viewer");
    assert_eq!(missing(&window, &["AGGREGATION", "namespaces", "get", "/healthz"]), Vec::<String>::new());
    assert!(shows_part(&window, "each one with fresnel.test/view=true"));
}

#[test]
fn a_role_binding_links_its_role_and_its_service_account_and_names_its_users() {
    let cluster = and(live("ams-prod", &of_kinds::<RoleBinding>()), &of_kinds::<Role>());
    let mut window = opened::<RoleBinding>(vec![cluster], "reader");
    let texts = ["Role", "Role reader", "SUBJECTS", "User", "alice", "Group", "devs", "ServiceAccount", "web", "kinds"];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    follow(&mut window, "Role reader");
    until(&mut window, "the panel of the Role", |window| in_panel(window, "RULES"));

    let window =
        opened::<ClusterRoleBinding>(vec![live("ams-prod", &of_kinds::<ClusterRoleBinding>())], "fresnel-viewer");
    assert_eq!(
        missing(&window, &["ClusterRole fresnel-viewer", "ServiceAccount", "web", "kinds"]),
        Vec::<String>::new()
    );
}

/// The page of the ServiceAccounts holds the full bindings for the panel. A store of them
/// that is not there yet is a note, not "None".
#[test]
fn a_service_account_lists_the_bindings_that_name_it_with_their_role() {
    let accounts = of_kinds::<ServiceAccount>();
    let bindings = and(live("ams-prod", &accounts), &of_kinds::<RoleBinding>());
    let mut window = opened::<ServiceAccount>(vec![and(bindings, &of_kinds::<ClusterRoleBinding>())], "web");
    let wanted = BTreeSet::from([Kind::ServiceAccount, Kind::RoleBinding, Kind::ClusterRoleBinding]);
    assert!(window.state().sent.contains(&Command::ShowKinds(wanted)));
    let texts = ["Mount token", "yes", "ROLE BINDINGS", "RoleBinding reader", "Role reader"];
    let texts = [&texts[..], &["ClusterRoleBinding fresnel-viewer", "ClusterRole fresnel-viewer"]].concat();
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    assert!(!in_panel(&window, "ClusterRoleBinding fresnel-reader"), "a binding of a user");
    follow(&mut window, "ClusterRoleBinding fresnel-viewer");
    until(&mut window, "the panel of the binding", |window| in_panel(window, "SUBJECTS"));

    let window = opened::<ServiceAccount>(vec![live("ams-prod", &accounts)], "default");
    assert!(shows(&window, "The RoleBindings are not loaded: their store starts."));
    assert_eq!(times_in_panel(&window, "None."), 1, "no list, no \"None\": the one is of the pods that use it");
}

/// A user who may not list the RoleBindings: the panel tells the reason and shows the
/// ClusterRoleBindings. With no binding of the other kind, it says no "None": a binding
/// that the user cannot see can name the account.
#[test]
fn a_service_account_whose_role_bindings_are_forbidden_tells_why_and_shows_the_other_bindings() {
    let refused = store(StoreState::Forbidden { reason: forbidden() }, &Vec::<Arc<RoleBinding>>::new());
    let row = and(live("ams-prod", &of_kinds::<ServiceAccount>()), &of_kinds::<ClusterRoleBinding>());
    let mut data = row.data.clone().expect("data");
    data.objects = data.objects.with(held(Snapshot { listed: false, ..refused }));
    let cluster = ClusterRow { data: Some(data), ..row };
    let window = opened::<ServiceAccount>(vec![cluster.clone()], "web");
    assert!(shows_part(&window, "The RoleBindings are not loaded: their store forbidden: pods is forbidden"));
    assert!(in_panel(&window, "ClusterRoleBinding fresnel-viewer"), "the bindings of the other kind");
    let window = opened::<ServiceAccount>(vec![cluster], "default");
    assert_eq!(times_in_panel(&window, "None."), 1, "the one is of the pods that use it");
}

#[test]
fn a_service_account_links_its_secrets_and_tells_the_mount_of_its_token() {
    let accounts = of_kinds::<ServiceAccount>();
    let robot = changed_object(&*accounts[0], |value| {
        value["metadata"]["name"] = "robot".into();
        value["automountServiceAccountToken"] = false.into();
        value["imagePullSecrets"] = json!([{"name": "registry"}]);
        value["secrets"] = json!([{"name": "robot-token"}]);
    });
    let mut window = opened::<ServiceAccount>(vec![live("ams-prod", &[robot])], "robot");
    assert_eq!(missing(&window, &["registry", "robot-token"]), Vec::<String>::new());
    assert!(shows_part(&window, "no: a pod mounts the token only when its spec asks"));
    follow(&mut window, "registry");
    until(&mut window, "the page of the Secrets", |window| {
        window.state().sent.contains(&Command::ShowKinds(BTreeSet::from([Kind::Secret])))
    });
}
