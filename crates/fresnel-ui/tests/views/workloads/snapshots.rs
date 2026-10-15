//! PNG files of the workload views in the dark theme, for a look by a person: the panel of
//! each kind, the confirm dialog of one object and of a bulk action, the kubectl commands,
//! a forbidden action and the Operations tab.

use egui::Modifiers;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};

use super::bulk::open_menu;
use super::scene::{access, allowed, cluster, hover, rules_without};
use crate::data::{pods, scene};
use crate::fake::{open_tab, view, window_with};
use crate::snapshots::{SCALE, panel_of, save, save_clean, tall};

#[test]
fn the_panel_of_a_pod_with_its_containers() {
    let mut window = tall(panel_of(vec![allowed("ams-prod", &pods())], "Pods", "half-done"), "CONTAINERS");
    save_clean(&mut window, "m3-pod");
}

#[test]
fn the_panel_of_a_deployment_with_its_pods_and_actions() {
    let window = panel_of(vec![allowed("ams-prod", &scene::<Deployment>())], "Deployments", "web");
    let mut window = tall(window, "DETAILS");
    save_clean(&mut window, "m3-deployment");
}

#[test]
fn the_panel_of_a_stateful_set_a_job_and_a_cron_job() {
    let window = panel_of(vec![allowed("ams-prod", &scene::<StatefulSet>())], "StatefulSets", "db");
    let mut window = tall(window, "PODS");
    save_clean(&mut window, "m3-stateful-set");
    let mut window = tall(panel_of(vec![allowed("ams-prod", &scene::<Job>())], "Jobs", "fails"), "DETAILS");
    save_clean(&mut window, "m3-job");
    let window = panel_of(vec![allowed("ams-prod", &scene::<CronJob>())], "CronJobs", "tick");
    let mut window = tall(window, "DETAILS");
    save_clean(&mut window, "m3-cron-job");
}

#[test]
fn the_confirm_dialog_of_a_scale() {
    let mut window = panel_of(vec![allowed("ams-prod", &scene::<Deployment>())], "Deployments", "web");
    window.get_by_role_and_label(Role::Button, "Scale…").click();
    window.run();
    save_clean(&mut window, "m3-confirm-scale");
}

#[test]
fn the_confirm_dialog_of_a_force_delete_of_a_pod_of_a_stateful_set() {
    let mut window = panel_of(vec![allowed("ams-prod", &pods())], "Pods", "db-0");
    window.get_by_role_and_label(Role::Button, "More").click();
    window.run();
    window.get_by_label("Force delete…").click();
    window.run();
    save_clean(&mut window, "m3-confirm-force-delete");
}

#[test]
fn the_dialog_of_a_bulk_restart_in_two_clusters() {
    let deployments = scene::<Deployment>();
    let clusters = vec![allowed("ams-prod", &deployments), allowed("fra-stage", &deployments)];
    let mut window = window_with(view(clusters), egui::Theme::Dark, SCALE);
    crate::fake::open_page(&mut window, "Deployments");
    crate::fake::until(&mut window, "the rows", |window| window.query_all_by_label("web").count() == 2);
    window.query_all_by_label("web").next().expect("a row").click();
    window.run();
    window.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    window.run();
    open_menu(&mut window);
    window.get_by_label("Restart the selected").click();
    window.run();
    save_clean(&mut window, "m3-bulk-restart");
}

#[test]
fn the_kubectl_commands_of_a_pod() {
    let mut window = panel_of(vec![allowed("ams-prod", &pods())], "Pods", "half-done");
    window.get_by_role_and_label(Role::Button, "More").click();
    window.run();
    window.get_by_label_contains("Copy kubectl command").click();
    window.run();
    save_clean(&mut window, "m3-kubectl-commands");
}

#[test]
fn the_actions_that_the_rules_forbid_are_off_with_the_reason() {
    let deployments = scene::<Deployment>();
    let rules = access(rules_without(&[("patch", "deployments/scale"), ("delete", "deployments")]));
    let mut window = panel_of(vec![cluster("ams-prod", &deployments, rules)], "Deployments", "web");
    hover(&mut window, Role::Button, "Scale…");
    save(&mut window, "m3-forbidden");
}

#[test]
fn the_operations_tab_with_each_state_of_a_job() {
    let mut window = super::operations::session_window(SCALE);
    open_tab(&mut window, "Operations");
    save_clean(&mut window, "m3-operations");
}
