//! The actions that open a terminal (plan section 12.8): Shell, Attach and Debug of a pod,
//! and Node shell of a node, from the header of the panel and the per-container buttons. A
//! shell and an attach open at once; a debug container and a node shell ask first.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
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

/// A shell and an attach open at once, with the first container of the pod, from the More
/// menu of the panel.
#[test]
fn a_shell_and_an_attach_of_a_pod_open_at_once_with_its_container() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    open(&mut window, "Pods", "db-0");
    menu(&mut window, "Terminal");
    assert_eq!(opened(&window), [on_pod("db-0", "db", Program::Shell)]);
    menu(&mut window, "Attach");
    assert_eq!(opened(&window), [on_pod("db-0", "db", Program::Shell), on_pod("db-0", "db", Program::Attach)]);
}

/// The menu of a row and the key of the command table reach the same actions: the menu opens
/// the panel of its row and an attach, and Cmd+Shift+T a shell.
#[test]
fn the_menu_of_a_row_and_a_key_open_a_terminal() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    crate::fake::open_page(&mut window, "Pods");
    window.get_by_role(Role::TextInput).click();
    window.run();
    window.get_by_role(Role::TextInput).type_text("db-0");
    window.run();
    crate::fake::until(&mut window, "the row of db-0", |window| window.query_by_label("db-0").is_some());
    window.get_by_label("db-0").click_secondary();
    window.run();
    window.get_by_role_and_label(Role::Button, "Attach").click();
    window.run();
    assert_eq!(opened(&window), [on_pod("db-0", "db", Program::Attach)]);
    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::T);
    window.run();
    assert_eq!(opened(&window), [on_pod("db-0", "db", Program::Attach), on_pod("db-0", "db", Program::Shell)]);
}

/// Plan section 10: a terminal that the rules forbid is off with its reason, and a button of
/// the container section gives that reason and opens nothing. The rules of the scene let
/// the user evict a pod, but they have no `pods/exec` and no `pods/attach`.
#[test]
fn a_forbidden_terminal_is_off_with_its_reason_and_its_button_opens_nothing() {
    use crate::workloads::scene::{access, cluster, rules_without};
    let mut window = window(view(vec![cluster("ams-prod", &pods(), access(rules_without(&[])))]));
    open(&mut window, "Pods", "db-0");
    window.get_by_role_and_label(Role::Button, "More").click();
    window.run();
    let off =
        |window: &Window, action| window.get_by_role_and_label(Role::Button, action).accesskit_node().is_disabled();
    assert!(off(&window, "Terminal") && off(&window, "Attach") && off(&window, "Debug container…"));
    let evict = window.query_all_by_role_and_label(Role::Button, "Evict…").collect::<Vec<_>>();
    assert!(!evict.is_empty() && evict.iter().all(|node| !node.accesskit_node().is_disabled()), "the rules permit it");
    window.key_press(Key::Escape);
    window.run();
    click(&mut window, "Shell of db");
    let namespace = pod("db-0").metadata.namespace.clone().unwrap_or_default();
    let reason = format!("Terminal did not run: Forbidden: no rule lets you create pods/exec db-0 in {namespace}.");
    assert!(crate::fake::shows(&window, &reason), "the note of the panel");
    assert_eq!(opened(&window), []);
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
