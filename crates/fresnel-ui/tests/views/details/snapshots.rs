//! PNG files of the detail views of milestone M7 in the dark theme, for a look by a person:
//! a panel of each group of the sidebar with the most sections.

use k8s_openapi::api::admissionregistration::v1::ValidatingWebhookConfiguration;
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::core::v1::{Service, ServiceAccount};
use k8s_openapi::api::networking::v1::{Ingress, NetworkPolicy};
use k8s_openapi::api::rbac::v1::{ClusterRoleBinding, RoleBinding};
use serde_json::json;

use super::and;
use super::cluster::namespaces;
use crate::data::{changed_object, events, of_kinds, scene};
use crate::kinds::scene::{live, nodes};
use crate::snapshots::{panel_of, save_clean, tall};

#[test]
fn the_panels_of_the_network_kinds() {
    let window = panel_of(vec![live("ams-prod", &scene::<Service>())], "Services", "web");
    save_clean(&mut tall(window, "DETAILS"), "m7-service");
    let window = panel_of(vec![live("ams-prod", &of_kinds::<Ingress>())], "Ingresses", "web");
    save_clean(&mut tall(window, "DETAILS"), "m7-ingress");
    let policy = changed_object(&*of_kinds::<NetworkPolicy>()[0], |value| {
        value["spec"]["ingress"] = json!([{"from": [{"namespaceSelector": {"matchLabels": {"team": "front"}}}],
            "ports": [{"port": 80}, {"port": 8000, "endPort": 9000}]}]);
    });
    let window = panel_of(vec![live("ams-prod", &[policy])], "NetworkPolicies", "web");
    save_clean(&mut tall(window, "DETAILS"), "m7-network-policy");
}

#[test]
fn the_panels_of_the_config_kinds() {
    let scalers = of_kinds::<HorizontalPodAutoscaler>();
    let window = panel_of(vec![live("ams-prod", &scalers)], "HorizontalPodAutoscalers", "web");
    save_clean(&mut tall(window, "DETAILS"), "m7-autoscaler");
    let hooks = of_kinds::<ValidatingWebhookConfiguration>();
    let window = panel_of(vec![live("ams-prod", &hooks)], "ValidatingWebhookConfigurations", "fresnel-validate");
    save_clean(&mut tall(window, "WEBHOOKS"), "m7-webhooks");
}

#[test]
fn the_panels_of_the_access_kinds() {
    let accounts = and(live("ams-prod", &of_kinds::<ServiceAccount>()), &of_kinds::<RoleBinding>());
    let window = panel_of(vec![and(accounts, &of_kinds::<ClusterRoleBinding>())], "ServiceAccounts", "web");
    save_clean(&mut tall(window, "DETAILS"), "m7-service-account");
    let window = panel_of(vec![live("ams-prod", &of_kinds::<RoleBinding>())], "RoleBindings", "reader");
    save_clean(&mut tall(window, "DETAILS"), "m7-role-binding");
}

#[test]
fn the_panels_of_the_cluster_kinds() {
    let node = nodes()[0].metadata.name.clone().expect("a name");
    let window = panel_of(vec![live("ams-prod", &nodes())], "Nodes", &node);
    save_clean(&mut tall(window, "CAPACITY"), "m7-node");
    let window = panel_of(vec![namespaces()], "Namespaces", "scene");
    save_clean(&mut tall(window, "RESOURCE QUOTAS"), "m7-namespace");
    let window = panel_of(vec![live("ams-prod", &events())], "Events", "crash.18de28919fa8e1d0");
    save_clean(&mut tall(window, "DETAILS"), "m7-event");
}
