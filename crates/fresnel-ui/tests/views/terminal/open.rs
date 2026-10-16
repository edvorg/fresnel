//! The actions that open a terminal (plan section 12.8): Shell, Attach and Debug of a pod,
//! and Node shell of a node, from the header of the panel and the per-container buttons. A
//! shell and an attach open at once; a debug container and a node shell ask first.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::Command;
use fresnel_core::store::ObjectKey;
use fresnel_core::terminal::{Program, TerminalRequest};

use crate::data::{live, pod, pods};
use crate::details::shows_part;
use crate::fake::{Window, shows, view, window};
use crate::kinds::scene::{nodes, uid_of};
use crate::workloads::scene::open;

/// Opens the "More" menu of the panel and clicks an action in it by its exact label, so it
/// does not match the per-container buttons of the container section.
fn menu(window: &mut Window, action: &str) {
    window.get_by_role_and_label(Role::Button, "More").click();
    window.run();
    window.get_by_role_and_label(Role::Button, action).click();
    window.run();
}

/// Clicks a dialog or section button of the panel, after it is on the screen.
fn click(window: &mut Window, label: &str) {
    window.get_by_role_and_label(Role::Button, label).scroll_to_me();
    window.run();
    window.get_by_role_and_label(Role::Button, label).click();
    window.run();
}

/// The requests of each `Command::OpenTerminal` that the views sent.
fn opened(window: &Window) -> Vec<TerminalRequest> {
    window
        .state()
        .sent
        .iter()
        .filter_map(|command| match command {
            Command::OpenTerminal(request) => Some(request.clone()),
            _ => None,
        })
        .collect()
}

/// The expected request of a terminal on a pod of `ams-prod`.
fn on_pod(name: &str, container: &str, program: Program) -> TerminalRequest {
    TerminalRequest {
        cluster: fresnel_core::cluster::ClusterId::new("ams-prod"),
        object: ObjectKey::new(pod(name).metadata.namespace.as_deref().unwrap_or_default(), name),
        uid: pod(name).metadata.uid.clone().expect("a UID").into(),
        container: container.into(),
        program,
    }
}

/// A shell and an attach open at once, with the default container of the pod. The header of
/// the panel has a button for each; the row menu and the keys reach the same action.
#[test]
fn a_shell_and_an_attach_of_a_pod_open_at_once_with_its_container() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    open(&mut window, "Pods", "db-0");
    menu(&mut window, "Terminal");
    assert_eq!(opened(&window), [on_pod("db-0", "db", Program::Shell)]);
    menu(&mut window, "Attach");
    assert_eq!(opened(&window), [on_pod("db-0", "db", Program::Shell), on_pod("db-0", "db", Program::Attach)]);
}

/// A pod of two containers: the container section has a Shell button for each, so the user
/// picks the container of the terminal.
#[test]
fn the_container_section_opens_a_shell_in_the_container_that_the_user_chose() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    open(&mut window, "Pods", "half-done");
    // The container section shows a Shell button for the container `main`.
    window.get_by_label("Shell of main").scroll_to_me();
    window.run();
    window.get_by_label("Shell of main").click();
    window.run();
    assert_eq!(opened(&window), [on_pod("half-done", "main", Program::Shell)]);
}

/// A debug container asks first: its dialog names the target and warns that the pod keeps
/// the container, and the confirm opens a terminal of the program Debug.
#[test]
fn a_debug_container_asks_first_and_opens_after_the_confirm() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    open(&mut window, "Pods", "db-0");
    menu(&mut window, "Debug container…");
    assert!(shows(&window, "Add a debug container"));
    assert!(shows_part(&window, "The pod keeps the container for its whole life"));
    assert!(opened(&window).is_empty(), "nothing opens before the confirm");
    click(&mut window, "Open the terminal");
    assert_eq!(opened(&window), [on_pod("db-0", "db", Program::Debug)]);
}

/// A node shell asks first and opens a terminal on the node, with no container.
#[test]
fn a_node_shell_asks_first_and_opens_a_terminal_on_the_node() {
    let node = nodes()[0].metadata.name.clone().expect("a name");
    let mut window = window(view(vec![crate::kinds::scene::live("ams-prod", &nodes())]));
    open(&mut window, "Nodes", &node);
    menu(&mut window, "Node shell…");
    assert!(shows(&window, "Open a shell on the node"));
    assert!(shows_part(&window, "The app deletes the pod when the tab closes."));
    click(&mut window, "Open the terminal");
    let request = TerminalRequest {
        cluster: fresnel_core::cluster::ClusterId::new("ams-prod"),
        object: ObjectKey::new("", &node),
        uid: uid_of(&nodes(), &node),
        container: "".into(),
        program: Program::NodeShell,
    };
    assert_eq!(opened(&window), [request]);
}

/// PNG files of the terminal actions: the container section of a pod with its Shell, Attach
/// and Debug buttons, and the confirm dialog of a node shell.
#[test]
fn the_container_buttons_and_the_node_shell_dialog() {
    use crate::snapshots::save_clean;
    let mut pods_window = window(view(vec![live("ams-prod", &pods())]));
    open(&mut pods_window, "Pods", "half-done");
    pods_window.get_by_label("Shell of main").scroll_to_me();
    pods_window.run();
    save_clean(&mut pods_window, "terminal-actions");

    let node = nodes()[0].metadata.name.clone().expect("a name");
    let mut node_window = window(view(vec![crate::kinds::scene::live("ams-prod", &nodes())]));
    open(&mut node_window, "Nodes", &node);
    menu(&mut node_window, "Node shell…");
    save_clean(&mut node_window, "terminal-node-shell");
}
