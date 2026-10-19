//! Plan section 11.6: a quit while a forward has its local port or a terminal is open asks
//! first: minimize, quit or cancel. A forward that stopped, or whose port is in use, has none,
//! and a terminal whose program ended is no reason. A close of the window (its button, Alt+F4,
//! the taskbar of Windows, the quit key of egui) and a quit of the app (the Quit item and the
//! Dock of macOS) take the same decision, and the setting `ask_before_quit` turns it off.

use egui::{Event, Key, Modifiers, ViewportCommand, ViewportEvent, ViewportId};
use fresnel_core::terminal::{SessionState, TerminalView};
use fresnel_ui::Quit;

use super::*;
use crate::fake::{logic, quit_app, shows};
use crate::settings::with_config;
use crate::terminal::{terminal, with_terminals};

/// Asks the window to close for one frame, as the close button, Alt+F4 and the taskbar of
/// Windows do (`WM_CLOSE`).
fn close(window: &mut Window) {
    window.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.push(ViewportEvent::Close);
    window.step();
}

fn commands(window: &Window) -> Vec<ViewportCommand> {
    window.output().viewport_output.get(&ViewportId::ROOT).map(|output| output.commands.clone()).unwrap_or_default()
}

#[test]
fn a_close_while_a_forward_runs_asks_and_quit_closes_and_minimize_keeps_the_window() {
    let mut window = tab_window(forwards());
    close(&mut window);
    assert!(commands(&window).contains(&ViewportCommand::CancelClose), "the close waits for the user");
    window.run();
    assert!(
        shows(&window, "3 forwards of the app run: a quit closes their local ports."),
        "the failed one has no port"
    );
    window.get_by_label("Minimize").click();
    window.step();
    assert!(commands(&window).contains(&ViewportCommand::Minimized(true)));
    window.run();
    assert!(!shows(&window, "Quit Fresnel?"));

    close(&mut window);
    window.run();
    window.get_by_label("Quit").click();
    window.step();
    assert!(commands(&window).contains(&ViewportCommand::Close), "the close goes on");
    close(&mut window);
    assert!(!commands(&window).contains(&ViewportCommand::CancelClose), "the user chose to quit");
}

#[test]
fn a_close_with_no_running_forward_goes_on_at_once() {
    let stopped = |id, state| {
        let rule = rule("ams-prod", Kind::Pod, "db-0", 5432, LocalPort::Fixed(5432));
        ForwardRow { id, saved: false, view: held(ForwardView::new(rule, state, now())) }
    };
    let failed = ForwardState::Failed("the local port 5432 is in use".to_owned());
    let mut window = tab_window(vec![stopped(1, ForwardState::Stopped), stopped(2, failed)]);
    close(&mut window);
    assert!(!commands(&window).contains(&ViewportCommand::CancelClose));
    window.run();
    assert!(!shows(&window, "Quit Fresnel?"));
}

/// A forward that the server refuses keeps its local port, so a close asks first.
#[test]
fn a_close_asks_while_a_refused_forward_keeps_its_port() {
    let rule = rule("ams-prod", Kind::Pod, "db-0", 5432, LocalPort::Fixed(5432));
    let refused = ForwardState::Failed("the server refuses the port forward: forbidden".to_owned());
    let view = ForwardView { addresses: addresses(&["127.0.0.1:5432"]), ..ForwardView::new(rule, refused, now()) };
    let mut window = tab_window(vec![ForwardRow { id: 1, saved: false, view: held(view) }]);
    close(&mut window);
    assert!(commands(&window).contains(&ViewportCommand::CancelClose), "the close waits for the user");
    window.run();
    assert!(shows(&window, "1 forward of the app runs: a quit closes its local port."));
}

/// A quit ends the program of each open terminal in its pod: the quit key of egui (Ctrl+Q on
/// Windows; the Quit item of macOS takes Cmd+Q first) is a close of the window, which asks.
#[test]
fn cmd_q_while_a_terminal_is_open_asks_first_and_an_ended_terminal_is_no_reason() {
    let (web, db, old) = (terminal(1, "web-1"), terminal(2, "db-0"), terminal(3, "web-0"));
    old.terminal.set_state(SessionState::Exited { code: 0 });
    let with = |terminals: &[TerminalView]| window(with_terminals(view(vec![live("ams-prod", &pods())]), terminals));
    let mut ended = with(std::slice::from_ref(&old));
    close(&mut ended);
    assert!(!commands(&ended).contains(&ViewportCommand::CancelClose), "the program of the terminal ended");

    let mut window = with(&[web, db, old]);
    // One frame: the commands of the frame of the key.
    let modifiers = Modifiers::COMMAND;
    window.event(Event::Key { key: Key::Q, physical_key: None, pressed: true, repeat: false, modifiers });
    window.step();
    assert!(commands(&window).contains(&ViewportCommand::Close), "the quit key asks the window to close");
    close(&mut window);
    assert!(commands(&window).contains(&ViewportCommand::CancelClose), "the close waits for the user");
    window.run();
    assert!(
        shows(&window, "Quit Fresnel?")
            && shows(&window, "2 terminals are open: a quit ends their programs in the pods.")
    );
    window.get_by_label("Cancel").click();
    window.run();
    assert!(!shows(&window, "Quit Fresnel?"));
}

#[test]
fn the_setting_ask_before_quit_turns_the_question_off() {
    let settings = "version = 1\n[safety]\nask_before_quit = false\n";
    let mut window = window(with_config(with_forwards(forwards()), settings));
    close(&mut window);
    assert!(!commands(&window).contains(&ViewportCommand::CancelClose), "no question");
    assert_eq!(quit_app(&window), Quit::Now, "no question at a quit of the app");
    window.run();
    assert!(!shows(&window, "Quit Fresnel?"));
}

/// A quit of the app (the Quit item, Cmd+Q or the Dock of macOS) with nothing to end ends the
/// app at once, and the shell shows nothing.
#[test]
fn a_quit_of_the_app_with_no_terminal_and_no_forward_ends_at_once() {
    let mut window = tab_window(Vec::new());
    assert_eq!(quit_app(&window), Quit::Now);
    window.run();
    assert!(!shows(&window, "Quit Fresnel?"));
}

/// The question of a quit of the app: each choice answers the quit that follows it. After
/// "Minimize" or "Cancel" the next quit asks again, also of the minimized window, which comes
/// back; after "Quit" the window closes and each quit goes through.
#[test]
fn a_quit_of_the_app_while_a_forward_runs_asks_again_after_minimize_and_cancel() {
    let mut window = tab_window(forwards());
    assert_eq!(quit_app(&window), Quit::Ask, "the app stays");
    window.step();
    assert!(commands(&window).contains(&ViewportCommand::Focus), "the window takes the focus");
    window.run();
    assert!(shows(&window, "3 forwards of the app run: a quit closes their local ports."));
    window.get_by_label("Minimize").click();
    window.step();
    assert!(commands(&window).contains(&ViewportCommand::Minimized(true)));
    window.run();
    assert!(!shows(&window, "Quit Fresnel?"));

    // Cmd+Q of a minimized window: eframe runs its logic alone, and the window comes back.
    assert_eq!(quit_app(&window), Quit::Ask);
    let mut input = egui::RawInput::default();
    input.viewports.entry(ViewportId::ROOT).or_default().minimized = Some(true);
    let back = logic(&mut window, &input);
    assert!(back.contains(&ViewportCommand::Minimized(false)) && back.contains(&ViewportCommand::Focus), "{back:?}");
    assert_eq!(logic(&mut window, &input), Vec::new(), "it comes back one time");
    window.run();
    assert!(shows(&window, "Quit Fresnel?"));
    window.get_by_label("Cancel").click();
    window.run();
    assert!(!shows(&window, "Quit Fresnel?"));

    assert_eq!(quit_app(&window), Quit::Ask, "a cancel answers one quit");
    window.run();
    window.get_by_label("Quit").click();
    window.step();
    assert!(commands(&window).contains(&ViewportCommand::Close), "the close goes on");
    assert_eq!(quit_app(&window), Quit::Now, "the user chose to quit");
}

/// eframe runs no frame for a minimized window, only the logic of the app: a close of it, as
/// from the taskbar of Windows, stops there, and the window comes back to show its question.
#[test]
fn a_close_of_a_minimized_window_stops_with_no_frame_and_the_window_comes_back_to_ask() {
    let mut window = tab_window(forwards());
    let mut input = egui::RawInput::default();
    let root = input.viewports.entry(ViewportId::ROOT).or_default();
    (root.minimized, root.events) = (Some(true), vec![ViewportEvent::Close]);
    let commands = logic(&mut window, &input);
    assert!(commands.contains(&ViewportCommand::CancelClose), "the close waits for the user");
    assert!(commands.contains(&ViewportCommand::Minimized(false)) && commands.contains(&ViewportCommand::Focus));
    window.run();
    assert!(shows(&window, "Quit Fresnel?"));

    // With no reason to ask, the close of a hidden window goes on.
    let mut stopped = tab_window(Vec::new());
    assert_eq!(logic(&mut stopped, &input), Vec::new(), "no terminal and no forward");
}
