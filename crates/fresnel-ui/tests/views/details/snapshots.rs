//! PNG files of the detail views of milestone M7 in the dark theme, for a look by a person:
//! a panel of each group of the sidebar with the most sections.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::ops::JobState;
use k8s_openapi::api::admissionregistration::v1::ValidatingWebhookConfiguration;
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::core::v1::{Service, ServiceAccount};
use k8s_openapi::api::networking::v1::{Ingress, NetworkPolicy};
use k8s_openapi::api::rbac::v1::{ClusterRoleBinding, RoleBinding};
use serde_json::json;

use super::and;
use super::cluster::namespaces;
use super::drain::{click, drain_job, node, with_nodes};
use crate::data::{changed_object, events, of_kinds, scene};
use crate::fake::{open_tab, view, window_with};
use crate::kinds::scene::{live, nodes};
use crate::snapshots::{SCALE, panel_of, save_clean, tall};
use crate::workloads::scene::jobs;

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
    let window = panel_of(vec![namespaces("ams-prod")], "Namespaces", "scene");
    save_clean(&mut tall(window, "RESOURCE QUOTAS"), "m7-namespace");
    let window = panel_of(vec![live("ams-prod", &events())], "Events", "crash.18de28919fa8e1d0");
    save_clean(&mut tall(window, "DETAILS"), "m7-event");
}

#[test]
fn the_drain_dialog_and_the_pods_of_a_drain_in_the_operations_tab() {
    let mut window = panel_of(vec![with_nodes(&nodes())], "Nodes", &node());
    click(&mut window, "Drain…");
    window.get_by_label("Ignore DaemonSets: their pods stay on the node").click();
    window.run();
    save_clean(&mut window, "m7-drain-dialog");
    let mut core = view(vec![with_nodes(&nodes())]);
    core.jobs = jobs(vec![drain_job(1, JobState::Pending)], 0);
    let mut window = window_with(core, egui::Theme::Dark, SCALE);
    window.set_size(egui::vec2(crate::fake::WINDOW.x, 1100.0));
    window.run();
    open_tab(&mut window, "Operations");
    // The dock grows to show the pods of the drain: a drag of its top edge.
    let edge = window.get_by_role_and_label(Role::Button, "Operations").rect().top() - 4.0;
    let start = egui::pos2(800.0, edge);
    window.hover_at(start);
    window.run();
    window.drag_at(start);
    window.run();
    window.hover_at(start - egui::vec2(0.0, 420.0));
    window.run();
    window.drop_at(start - egui::vec2(0.0, 420.0));
    window.run();
    save_clean(&mut window, "m7-drain-job");
}
