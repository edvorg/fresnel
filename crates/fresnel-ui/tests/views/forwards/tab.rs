//! The Forwards tab: a line of each forward with its state, its target, its address and its
//! pod, the chosen one in full with its buttons, the counts of the status bar and the key of
//! the tab.

use egui_kittest::kittest::NodeT as _;
use fresnel_core::config::forwards::{forget_edit, save_edits};

use super::*;
use crate::fake::{absent, in_one_frame, press, shows};
use crate::logs::copied;

/// The URL that the views asked the system to open in the last frame.
fn opened_url(window: &Window) -> Option<String> {
    window.output().platform_output.commands.iter().find_map(|command| match command {
        egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
        _ => None,
    })
}

/// Clicks a widget after a scroll to it: the panel of the chosen forward scrolls in a short
/// dock.
fn click(window: &mut Window, label: &str) {
    window.get_by_label(label).scroll_to_me();
    window.run();
    window.get_by_label(label).click();
    window.run();
}

/// The list has one short line of each forward; the panel of the chosen one has its full
/// state, its addresses, its counts, the age of the data of its target and its last error.
#[test]
fn the_tab_lists_each_forward_and_shows_the_chosen_one_in_full() {
    let mut window = tab_window(forwards());
    let list = [
        "4 forwards: 1 active, 1 degraded, 1 failed, 1 reconnecting",
        "active",
        "reconnecting, next in 4 s",
        "failed",
        "degraded",
        "Service shop/web port 80",
        "Deployment shop/api port 9090",
        "127.0.0.1:8080",
        "127.0.0.1:49152",
        "web-6888b778b6-75md7:8080",
        "fra-prod",
    ];
    assert_eq!(absent(&window, &list), Vec::<&str>::new());
    // The first forward is the chosen one.
    let web = ["127.0.0.1:8080, [::1]:8080", "2 open, 14 in all", "12Mi sent, 300Ki received", "current"];
    assert_eq!(absent(&window, &web), Vec::<&str>::new());
    assert!(shows(&window, "yes, in config.toml"), "a saved forward");

    click(&mut window, "Forward of Deployment shop/api port 9090 in fra-prod");
    let api = [
        "reconnecting, attempt 3, next in 4 s: requests: connect: no answer in 10s",
        "0 open, 3 in all, 1 reset",
        "old for 48 s: the Pods are not live",
        "12 s ago: a tunnel broke: silent: no frame for 15.0 s",
    ];
    assert_eq!(absent(&window, &api), Vec::<&str>::new());
    click(&mut window, "Forward of Pod shop/db-0 port 5432 in ams-prod");
    assert!(shows(&window, "failed: the local port 5432 is in use: an other program listens on 127.0.0.1:5432"));
    click(&mut window, "Forward of StatefulSet shop/cache port 6379 in ams-prod");
    assert!(shows(&window, "degraded: the StatefulSet shop/cache has no pod that runs"));
}

#[test]
fn an_empty_tab_tells_how_a_forward_starts() {
    let window = tab_window(Vec::new());
    assert!(shows(&window, "No forward yet. \"Forward a port\" of a pod, a Service or a workload starts one."));
    assert!(!window.query_all_by_label_contains("forwards:").any(|_| true), "no count in the status bar");
}

#[test]
fn the_buttons_of_the_chosen_forward_send_its_commands_and_copy_and_open_its_address() {
    let rows = forwards();
    let rules: Vec<ForwardRule> = rows.iter().map(|row| row.view.rule.clone()).collect();
    let mut window = tab_window(rows);
    // The first forward is the chosen one: it runs and is saved.
    let web = "Service shop/web port 80 in ams-prod";
    window.get_by_label(&format!("Copy address: {web}")).scroll_to_me();
    window.run();
    window.get_by_label(&format!("Copy address: {web}")).click();
    window.step();
    assert_eq!(copied(&window).as_deref(), Some("127.0.0.1:8080"));
    window.get_by_label(&format!("Open in browser: {web}")).click();
    window.step();
    assert_eq!(opened_url(&window).as_deref(), Some("http://127.0.0.1:8080/"));
    click(&mut window, &format!("Stop: {web}"));
    click(&mut window, &format!("Forget: {web}"));
    assert!(
        window.get_by_label(&format!("Remove: {web}")).accesskit_node().is_disabled(),
        "a saved forward stays in the list"
    );
    assert!(
        window.get_by_label(&format!("Retry now: {web}")).accesskit_node().is_disabled(),
        "an active forward needs no retry"
    );
    assert_eq!(sent(&mut window), [Command::StopForward(1), Command::EditSettings(vec![forget_edit(&rules[0])])]);

    click(&mut window, "Forward of Deployment shop/api port 9090 in fra-prod");
    click(&mut window, "Retry now: Deployment shop/api port 9090 in fra-prod");
    let db = "Pod shop/db-0 port 5432 in ams-prod";
    click(&mut window, &format!("Forward of {db}"));
    assert!(
        window.get_by_label(&format!("Copy address: {db}")).accesskit_node().is_disabled(),
        "a failed forward has no address"
    );
    click(&mut window, &format!("Start: {db}"));
    click(&mut window, &format!("Save: {db}"));
    click(&mut window, &format!("Remove: {db}"));
    let commands = [
        Command::RetryForward(2),
        Command::RestartForward(3),
        Command::EditSettings(save_edits(&rules[2])),
        Command::RemoveForward(3),
    ];
    assert_eq!(sent(&mut window), commands);
}

/// The status bar counts the forwards of each state in the tone of the worst one, and a
/// click on it, or its key, opens the tab.
#[test]
fn the_status_bar_and_the_key_open_the_tab() {
    let mut window = window(with_forwards(forwards()));
    let count = "4 forwards: 1 active, 1 degraded, 1 failed, 1 reconnecting";
    assert!(shows(&window, count) && !shows(&window, "127.0.0.1:8080, [::1]:8080"), "the Journal tab");
    window.get_by_label(count).click();
    window.run();
    assert!(shows(&window, "127.0.0.1:8080, [::1]:8080"));

    window.get_by_label("Journal").click();
    window.run();
    in_one_frame(&mut window, press(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::F));
    window.run();
    assert!(shows(&window, "127.0.0.1:8080, [::1]:8080"), "Cmd+Shift+F opens the tab");
}
