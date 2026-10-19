//! Plan section 11.6: a close of the window while a forward has its local port or a terminal
//! is open asks first: minimize, quit or cancel. A forward that stopped, or whose port is in
//! use, has none, and a terminal whose program ended is no reason. Cmd+Q is a close too, and
//! the setting `ask_before_quit` turns the question off.

use egui::{Event, Key, Modifiers, ViewportCommand, ViewportEvent, ViewportId};
use fresnel_core::terminal::{SessionState, TerminalView};

use super::*;
use crate::fake::shows;
use crate::settings::with_config;
use crate::terminal::{terminal, with_terminals};

/// Asks the window to close, as the close button of the system does, for one frame.
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

/// A quit ends the program of each open terminal in its pod: Cmd+Q is a close of the window,
/// which asks first.
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
    assert!(commands(&window).contains(&ViewportCommand::Close), "Cmd+Q asks the window to close");
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
    window.run();
    assert!(!shows(&window, "Quit Fresnel?"));
}
