//! The bulk actions on the selected rows (plan section 10): scale, restart and delete. The
//! dialog lists each object with its cluster, and each object gets its own order, so each
//! one reaches its own cluster (plan section 6, weakness 9).

use egui::Modifiers;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::ops::{JobState, Operation, Order};
use k8s_openapi::api::apps::v1::Deployment;

use super::scene::{access, allowed, cluster, hover, job, jobs, rules_without, runs, target};
use crate::data::scene;
use crate::fake::{Window, shows, until, view, window};

/// The places of the rows of the objects with this name, from the top. The heading of the
/// detail panel has the name too; it is right of the table.
fn rows(window: &Window, name: &str) -> Vec<egui::Rect> {
    let panel = window.query_by_label("Overview").map_or(f32::INFINITY, |tab| tab.rect().left() - 2.0);
    let mut rects: Vec<egui::Rect> =
        window.query_all_by_label(name).map(|node| node.rect()).filter(|rect| rect.right() < panel).collect();
    rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
    rects
}

/// Selects the row of `web` of each cluster: a click, then a click with Cmd.
fn select_two(window: &mut Window) {
    crate::fake::open_page(window, "Deployments");
    until(window, "the rows of two clusters", |window| rows(window, "web").len() == 2);
    for (index, modifiers) in [(0, Modifiers::NONE), (1, Modifiers::COMMAND)] {
        let place = rows(window, "web").get(index).copied().expect("a row");
        let row = window.query_all_by_label("web").find(|node| node.rect() == place).expect("the row");
        row.click_modifiers(modifiers);
        window.run();
    }
    assert!(shows(window, "2 selected"));
}

/// Opens the menu of the selected rows: the button "2 selected" or so.
pub(super) fn open_menu(window: &mut Window) {
    let menu = window
        .query_all_by_role(Role::Button)
        .find(|node| node.value().is_some_and(|text| text.ends_with(" selected")));
    let menu = menu.or_else(|| {
        window.query_all_by_label_contains(" selected").find(|node| node.accesskit_node().role() == Role::Button)
    });
    menu.expect("the menu of the selected rows").click();
    window.run();
}

#[test]
fn a_bulk_restart_lists_each_object_with_its_cluster_and_sends_one_order_to_each_cluster() {
    let deployments = scene::<Deployment>();
    let mut window = window(view(vec![allowed("ams-prod", &deployments), allowed("fra-stage", &deployments)]));
    select_two(&mut window);
    open_menu(&mut window);
    window.get_by_label("Restart the selected").click();
    window.run();
    assert!(shows(&window, "Restart: 2 Deployments"));
    for cluster in ["ams-prod", "fra-stage"] {
        assert!(window.query_all_by_label(cluster).count() >= 2, "the dialog names {cluster}");
    }
    window.get_by_label("Confirm Restart").click();
    window.run();
    let order = |cluster| Order::new(target(cluster, &deployments, "web"), Operation::Restart).expect("an order");
    assert_eq!(runs(&window), [order("ams-prod"), order("fra-stage")]);
    assert!(shows(&window, "Started 2 jobs: Restart. The Operations tab shows them."));
}

#[test]
fn a_bulk_scale_sets_one_count_for_each_object() {
    let deployments = scene::<Deployment>();
    let mut window = window(view(vec![allowed("ams-prod", &deployments), allowed("fra-stage", &deployments)]));
    select_two(&mut window);
    open_menu(&mut window);
    window.get_by_label("Scale the selected").click();
    window.run();
    let count = window.get_by_role_and_label(Role::TextInput, "Count of pods").value();
    assert_eq!(count.as_deref(), Some("2"), "both have 2 pods");
    window.get_by_label("Confirm Scale").click();
    window.run();
    let order = |cluster| Order::new(target(cluster, &deployments, "web"), Operation::Scale(2)).expect("an order");
    assert_eq!(runs(&window), [order("ams-prod"), order("fra-stage")]);
}

/// One cluster forbids the delete: the bulk button is off, and its reason names the object.
#[test]
fn a_bulk_action_that_one_cluster_forbids_is_off_with_the_object_in_its_reason() {
    let deployments = scene::<Deployment>();
    let denied = access(rules_without(&[("delete", "deployments")]));
    let clusters = vec![allowed("ams-prod", &deployments), cluster("fra-stage", &deployments, denied)];
    let mut window = window(view(clusters));
    select_two(&mut window);
    open_menu(&mut window);
    let delete = window.get_by_label("Delete the selected");
    assert!(delete.accesskit_node().is_disabled());
    assert!(!window.get_by_label("Restart the selected").accesskit_node().is_disabled());
    hover(&mut window, Role::Button, "Delete the selected");
    let reason =
        "Deployment scene/web in fra-stage: Forbidden: no rule lets you delete deployments (apps) web in scene.";
    assert!(shows(&window, reason));
    assert!(window.query_by_role_and_label(Role::Button, "Confirm Delete").is_none(), "no dialog");
}

/// The core keeps each job that runs, at most 500: a bulk action on more objects than the
/// jobs that it can start now would end the rest as not sent, with only a journal entry. The
/// menu and the dialog refuse it with the reason, and the room of a job that ended counts.
#[test]
fn a_bulk_action_on_more_objects_than_the_jobs_that_the_core_can_start_is_off() {
    let deployments = scene::<Deployment>();
    let web = target("ams-prod", &deployments, "web");
    let running = |count: u64| {
        let each = (1..=count).map(|number| job(number, web.clone(), Operation::RolloutStatus, JobState::Pending, &[]));
        jobs(each.collect(), 0)
    };
    let mut session = view(vec![allowed("ams-prod", &deployments), allowed("fra-stage", &deployments)]);
    session.jobs = running(499);
    let mut window = window(session);
    select_two(&mut window);
    open_menu(&mut window);
    assert!(window.get_by_label("Restart the selected").accesskit_node().is_disabled());
    hover(&mut window, Role::Button, "Restart the selected");
    let reason =
        "The action needs 2 jobs and the app can start 1 job now. Select fewer objects, or wait for the jobs that run.";
    assert!(shows(&window, reason));

    window.state_mut().view.jobs = running(498);
    window.run();
    window.get_by_label("Restart the selected").click();
    window.run();
    assert!(!window.get_by_label("Confirm Restart").accesskit_node().is_disabled());
    // More jobs start while the dialog asks.
    window.state_mut().view.jobs = running(499);
    window.run();
    assert!(window.get_by_label("Confirm Restart").accesskit_node().is_disabled());
    assert!(shows(&window, reason));
    window.get_by_label("Confirm Restart").click();
    window.run();
    assert_eq!(runs(&window), Vec::<Order>::new());
}
