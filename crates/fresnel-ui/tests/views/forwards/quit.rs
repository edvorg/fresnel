//! Plan section 11.6: a close of the window while a forward has its local port asks first:
//! minimize, quit or cancel. A forward that stopped, or whose port is in use, has none.

use egui::{ViewportCommand, ViewportEvent, ViewportId};

use super::*;
use crate::fake::shows;

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
    assert!(shows(&window, "1 forward of the app run: a quit closes their local ports."));
}
