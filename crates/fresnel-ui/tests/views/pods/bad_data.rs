//! Data that a healthy API server does not give. The view stays correct with it: each
//! row is one object.

use egui_kittest::kittest::Queryable as _;

use crate::data::{changed, live, pod, renamed};
use crate::fake::{shows, view, window};

const DELETED: &str = "This pod is deleted. The panel shows its last state.";

/// An object of the API has a UID, and no other object of its cluster has the same one.
/// A row is one object also when a server does not hold that rule.
#[test]
fn pods_with_no_uid_and_pods_with_one_uid_are_objects_of_their_own() {
    let no_uid = |name: &str| {
        changed(&pod(name), |value| {
            value["metadata"].as_object_mut().expect("the metadata").remove("uid");
        })
    };
    let uid = pod("crash").metadata.uid.clone().expect("a UID");
    let pods = [no_uid("db-0"), no_uid("db-1"), pod("crash"), renamed(&pod("crash"), "crash-twin", &uid)];
    let mut window = window(view(vec![live("ams-prod", &pods)]));

    window.get_by_label("db-1").click();
    window.run();
    assert!(shows(&window, "apps.kubernetes.io/pod-index=1"), "the panel shows the pod of the click");
    assert!(!shows(&window, DELETED), "the store has the pod");
    assert!(window.query_all_by_label_contains(" selected").next().is_none(), "one pod is selected, not two");
    window.get_by_label("db-0").click_modifiers(egui::Modifiers::COMMAND);
    window.run();
    assert!(shows(&window, "2 selected"));

    window.get_by_label("crash-twin").click();
    window.run();
    assert_eq!(window.query_all_by_label("crash-twin").count(), 2, "the row and the heading of the panel");
    assert!(!shows(&window, DELETED) && window.query_all_by_label_contains(" selected").next().is_none());
    // The row, and not the container of the same name in the panel.
    let row = window.query_all_by_label("crash").min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()));
    row.expect("the row of crash").click_modifiers(egui::Modifiers::COMMAND);
    window.run();
    assert!(shows(&window, "2 selected"));

    // One of two pods with no UID goes away. The other one stays selected.
    window.get_by_label("db-0").click();
    window.get_by_label("db-1").click_modifiers(egui::Modifiers::COMMAND);
    window.run();
    assert!(shows(&window, "2 selected"));
    window.state_mut().view = view(vec![live("ams-prod", &pods[1..])]);
    crate::fake::until(&mut window, "the rows with no db-0", |window| shows(window, "3 pods"));
    assert!(window.query_all_by_label_contains(" selected").next().is_none(), "db-1 only");
    assert!(shows(&window, "apps.kubernetes.io/pod-index=1") && !shows(&window, DELETED), "the panel of db-1 stays");
}
