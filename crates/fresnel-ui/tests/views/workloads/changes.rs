//! Changes of the cluster while a confirm dialog is open: the dialog decides each target in
//! each frame from its store, so an order never goes to an object that the user did not see.

use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::ops::{Operation, Order};
use k8s_openapi::api::apps::v1::Deployment;

use super::scene::{allowed, opened, runs, target};
use crate::data::scene;
use crate::fake::{Window, shows, view};

/// The Deployments of the scene, with `web` changed by `change`.
fn with_web(change: impl FnOnce(&mut Deployment)) -> Vec<Arc<Deployment>> {
    let mut deployments = scene::<Deployment>();
    let web = deployments.iter_mut().find(|object| object.metadata.name.as_deref() == Some("web")).expect("web");
    change(Arc::make_mut(web));
    deployments
}

/// The cluster `ams-prod` has these Deployments from the next frame on.
fn store(window: &mut Window, deployments: &[Arc<Deployment>]) {
    window.state_mut().view = view(vec![allowed("ams-prod", deployments)]);
    window.run();
}

fn confirm_is_off(window: &Window, action: &str) -> bool {
    window.get_by_label(&format!("Confirm {action}")).accesskit_node().is_disabled()
}

/// The Deployment goes away while the dialog of a scale is open, and a new Deployment with
/// its name comes: the scale would set the count of pods of an object that the user did not
/// see, so the dialog keeps its button off. The object of the dialog of before comes back:
/// the button is on, and the order names that object.
#[test]
fn a_target_that_goes_away_or_that_a_new_object_replaces_gets_no_order() {
    let deployments = scene::<Deployment>();
    let mut window = opened::<Deployment>(vec![allowed("ams-prod", &deployments)], "web");
    window.get_by_role_and_label(Role::Button, "Scale…").click();
    window.run();
    assert!(!confirm_is_off(&window, "Scale"));

    let gone: Vec<Arc<Deployment>> =
        deployments.iter().filter(|object| object.metadata.name.as_deref() != Some("web")).cloned().collect();
    store(&mut window, &gone);
    assert!(confirm_is_off(&window, "Scale"));
    assert!(shows(&window, "Deployment scene/web in ams-prod: It is not in its store now."));

    let new = with_web(|web| web.metadata.uid = Some("a-new-object".to_owned()));
    store(&mut window, &new);
    assert!(confirm_is_off(&window, "Scale"));
    assert!(shows(&window, "Deployment scene/web in ams-prod: A new object with the same name took its place."));
    window.get_by_label("Confirm Scale").click();
    window.run();
    assert_eq!(runs(&window), Vec::<Order>::new(), "no order for the new object");

    store(&mut window, &deployments);
    assert!(!confirm_is_off(&window, "Scale"));
    window.get_by_label("Confirm Scale").click();
    window.run();
    let web = target("ams-prod", &deployments, "web");
    assert_eq!(runs(&window), [Order::new(web, Operation::Scale(2)).expect("an order")]);
}

/// The dialog reads the state of its object in each frame: a Deployment that someone pauses
/// while the dialog of a restart is open cannot restart, as kubectl says.
#[test]
fn a_deployment_that_someone_pauses_while_its_restart_asks_cannot_restart() {
    let deployments = scene::<Deployment>();
    let mut window = opened::<Deployment>(vec![allowed("ams-prod", &deployments)], "web");
    window.get_by_role_and_label(Role::Button, "Restart…").click();
    window.run();
    assert!(!confirm_is_off(&window, "Restart"));
    let paused = with_web(|web| web.spec.as_mut().expect("a spec").paused = Some(true));
    store(&mut window, &paused);
    assert!(confirm_is_off(&window, "Restart"));
    assert!(shows(
        &window,
        "Deployment scene/web in ams-prod: can't restart paused deployment (run rollout resume first)"
    ));
}
