//! The top bar, the status bar and the tabs.

use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::{Command, Startup};
use fresnel_core::cluster::{ClusterId, LinkState};
use fresnel_core::journal::Level;
use fresnel_core::net::ChannelState;

use crate::data::{live, pods};
use crate::fake::{
    Window, connected, connections_window, error, journal, live_channels, live_status, new_channel, off, open_page,
    open_tab, row, shows, status, view, window,
};

fn select(cluster: &str, selected: bool) -> Command {
    Command::Select { cluster: ClusterId::new(cluster), selected }
}

/// True when the switch of a cluster in the top bar is on.
fn is_selected(window: &Window, cluster: &str) -> bool {
    let chip = window.get_by_role_and_label(egui::accesskit::Role::CheckBox, cluster);
    chip.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

#[test]
fn the_top_bar_has_a_switch_for_each_cluster_and_a_click_sends_the_selection() {
    let clusters = vec![connected("ams-prod", live_status()), off("fra-stage"), off("nyc-prod")];
    let mut window = window(view(clusters));
    assert_eq!(
        ["ams-prod", "fra-stage", "nyc-prod"].map(|cluster| is_selected(&window, cluster)),
        [true, false, false]
    );

    window.get_by_role_and_label(egui::accesskit::Role::CheckBox, "fra-stage").click();
    window.run();
    window.get_by_role_and_label(egui::accesskit::Role::CheckBox, "ams-prod").click();
    window.run();
    assert_eq!(window.state().sent, [select("fra-stage", true), select("ams-prod", false)]);
    assert!(!is_selected(&window, "fra-stage"), "the view changes only when the core changes its state");
}

#[test]
fn all_and_none_send_a_command_for_each_cluster_that_changes() {
    let clusters = vec![connected("ams-prod", live_status()), off("fra-stage"), off("nyc-prod")];
    let mut window = window(view(clusters));
    window.get_by_label("Select all clusters").click();
    window.run();
    assert_eq!(window.state().sent, [select("fra-stage", true), select("nyc-prod", true)]);

    window.state_mut().sent.clear();
    window.get_by_label("Select no cluster").click();
    window.run();
    assert_eq!(window.state().sent, [select("ams-prod", false)]);
}

#[test]
fn the_top_bar_tells_how_far_the_start_is_while_it_has_no_cluster() {
    let texts = [
        (Startup::Waiting, "The login shell gives its environment…"),
        (Startup::Environment, "The login shell gives its environment…"),
        (Startup::Kubeconfig, "The kubeconfig files load…"),
        (Startup::Ready, "The kubeconfig files have no context."),
    ];
    for (startup, text) in texts {
        let mut starting = view(Vec::new());
        starting.startup = startup;
        let window = window(starting);
        assert!(shows(&window, text), "{startup:?}");
        assert!(shows(&window, "no cluster is selected"), "the status bar");
        assert!(!shows(&window, "Select all clusters"));
    }
}

#[test]
fn the_status_bar_has_one_state_for_each_selected_cluster_and_the_counts_of_the_journal() {
    let connecting = status([(); 3].map(|()| new_channel(ChannelState::Connecting { attempt: 1 })), None);
    let mut lost = live_channels();
    lost[2].state = ChannelState::Backoff { failures: 2, pause: crate::fake::SECOND };
    let clusters = vec![
        connected("ams-prod", live_status()),
        connected("fra-stage", connecting),
        connected("sgp-prod", status(lost, None)),
        row("late", LinkState::Failed(error("no server"))),
        off("nyc-prod"),
    ];
    let mut busy = view(clusters);
    busy.journal =
        journal(&[(Level::Error, "late", "x"), (Level::Error, "late", "y"), (Level::Warning, "app", "z")], 0);
    let mut window = window(busy);

    for state in ["ams-prod: live", "fra-stage: connecting", "sgp-prod: no connection", "late: failed"] {
        assert!(shows(&window, state), "{state}");
    }
    assert!(!window.query_all_by_label_contains("nyc-prod:").any(|_| true), "a cluster that is not selected");

    // A click on the counts opens the journal, and a click on a cluster opens its card.
    window.get_by_label("2 errors   1 warning").click();
    window.run();
    assert!(shows(&window, "Entry") && !shows(&window, "Channel"));
    window.get_by_label("ams-prod: live").click();
    window.run();
    assert!(shows(&window, "Channel") && !shows(&window, "Entry"));
    assert_eq!(window.state().sent, [], "a tab change is no command");
}

/// Design rule 1: the UI shows each state. The counts of the journal keep their place when
/// the selected clusters need more than the width of the window.
#[test]
fn many_selected_clusters_scroll_and_do_not_push_the_counts_out_of_the_window() {
    let clusters = (0..30).map(|index| connected(&format!("cluster-number-{index}"), live_status())).collect();
    let mut window = window(view(clusters));
    let counts = window.get_by_label("0 errors   0 warnings");
    assert!(counts.rect().right() <= 1280.0, "the counts are in the window: {:?}", counts.rect());
    counts.click();
    window.run();
    assert!(shows(&window, "Entry"), "a click on the counts opens the journal");
    window.get_by_label("cluster-number-0: live").click();
    window.run();
    assert!(shows(&window, "Channel"), "a click on a cluster opens its card");
}

/// A window that nobody uses must not use the processor: the views ask for a repaint
/// only for the age of an answer, one time in each second.
#[test]
fn an_idle_frame_asks_for_no_repaint_but_the_one_for_the_age_of_an_answer() {
    let repaint_delay = |window: &Window| {
        let output = window.output().viewport_output.get(&egui::ViewportId::ROOT).expect("the root viewport");
        output.repaint_delay
    };
    let mut idle = window(view(vec![off("ams-prod")]));
    idle.run();
    assert_eq!(repaint_delay(&idle), std::time::Duration::MAX);
    // A connected cluster with no pod: the Pods page has no age that changes.
    let mut no_pods = window(view(vec![live("ams-prod", &[])]));
    no_pods.run();
    assert_eq!(repaint_delay(&no_pods), std::time::Duration::MAX);

    let one_second = |window: &Window, what: &str| {
        let delay = repaint_delay(window);
        assert!(delay > std::time::Duration::from_millis(500) && delay <= crate::fake::SECOND, "{what}: {delay:?}");
    };
    let mut with_pods = window(view(vec![live("ams-prod", &pods())]));
    with_pods.run();
    one_second(&with_pods, "the age of a pod");
    let mut connections = connections_window(view(vec![connected("ams-prod", live_status())]));
    connections.run();
    one_second(&connections, "the age of an answer");
}

/// The user can make the window very small, and a display can be very large.
#[test]
fn each_page_and_each_tab_draws_at_each_window_size() {
    let clusters = vec![live("ams-prod", &pods()), connected("sgp-prod", live_status()), off("fra-stage")];
    let mut window = window(view(clusters));
    window.get_by_label("crash").click();
    window.run();
    for (page, tab) in [("Pods", "Journal"), ("Pods", "Connections"), ("Connections", "Connections")] {
        window.set_size(egui::vec2(1280.0, 800.0));
        window.run();
        open_page(&mut window, page);
        open_tab(&mut window, tab);
        for size in [[0.0, 0.0], [1.0, 1.0], [160.0, 90.0], [3840.0, 2160.0]] {
            window.set_size(size.into());
            window.run();
        }
    }
}
