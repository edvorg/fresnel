//! The Pods view while the window shows some other page (plan section 7.3): a view that
//! is not painted holds no rows.

use std::sync::Arc;

use crate::data::{live, pod, pods, renamed};
use crate::fake::{open_page, shows, until, view, window};

#[test]
fn the_view_holds_no_pod_and_no_selection_while_the_window_shows_an_other_page() {
    // Only this test, the store of the fake core and the view hold this pod.
    let held = renamed(&pod("crash"), "held", "the-uid-of-the-held-pod");
    let mut window = window(view(vec![live("ams-prod", std::slice::from_ref(&held))]));
    {
        use egui_kittest::kittest::Queryable as _;
        window.get_by_label("held").click();
    }
    window.run();
    assert!(shows(&window, "Close the details"), "the rows, the selection and the panel hold the pod");

    open_page(&mut window, "Connections");
    // The store has other pods now. A view that kept its rows would be the last holder
    // of the old ones for all the time on the other page.
    window.state_mut().view = view(vec![live("ams-prod", &pods())]);
    until(&mut window, "the view gave up each pointer to the pod", |_| Arc::strong_count(&held) == 1);

    open_page(&mut window, "Pods");
    until(&mut window, "the rows of the store of now", |window| shows(window, "25 pods"));
    assert!(!shows(&window, "Close the details"), "a view that was not painted has no selection");
}
