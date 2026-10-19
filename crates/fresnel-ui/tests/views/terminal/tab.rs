//! The tabs of the terminals in the dock, and the line of the state of each session.

use std::time::Duration;

use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::Command;
use fresnel_core::terminal::SessionState;

use super::{opened, terminal, title, with_terminals};
use crate::fake::{Window, now, open_tab, shows, until};

#[test]
fn each_terminal_has_a_tab_with_the_state_of_its_session_in_words() {
    let (web, db) = (terminal(1, "web-1"), terminal(2, "db-0"));
    let mut window = opened(&[web.clone(), db.clone()]);
    assert!(shows(&window, &title(&web)) && shows(&window, &title(&db)), "a tab for each terminal");
    assert!(shows(&window, "Connecting"), "the state of a session that did not open yet");
    let size = web.terminal.size();
    assert!(shows(&window, &format!("{} × {}", size.columns, size.rows)), "the size of the grid");

    let states = [
        (SessionState::Open, "Open"),
        (SessionState::InputWaits { since: now() - Duration::from_secs(12) }, "The program reads no input since 12 s"),
        (SessionState::Exited { code: 130 }, "Ended with exit code 130"),
        (SessionState::Failed { reason: "the connection was reset".to_owned() }, "Failed: the connection was reset"),
    ];
    for (state, words) in states {
        web.terminal.set_state(state);
        until(&mut window, words, |window| shows(window, words));
    }

    web.terminal.feed(b"\x1b]0;top - web-1\x07");
    until(&mut window, "the title of the program", |window| shows(window, "“top - web-1”"));
    open_tab(&mut window, &title(&db));
    assert!(shows(&window, "Connecting") && !shows(&window, "“top - web-1”"), "the line of the other terminal");
}

#[test]
fn a_close_asks_the_core_and_a_terminal_that_left_the_core_takes_its_tab_with_it() {
    let web = terminal(1, "web-1");
    let mut window = opened(std::slice::from_ref(&web));
    window.get_by_label(&format!("Close {}", title(&web))).click();
    window.run();
    assert!(window.state().sent.contains(&Command::CloseTerminal(1)));

    let view = window.state().view.clone();
    window.state_mut().view = with_terminals(view, &[]);
    window.run();
    assert!(!shows(&window, &title(&web)));
    assert!(window.query_by_label("Terminal").is_none(), "the dock shows an other tab");
}

/// A terminal that the core opens while the app runs gets its tab at once, as a new log
/// view does; a terminal that is already there at the start does not take the focus.
#[test]
fn a_terminal_that_appears_while_the_app_runs_opens_its_tab() {
    let web = terminal(1, "web-1");
    let mut window = opened(std::slice::from_ref(&web));
    assert!(shows(&window, "Connecting"), "the dock shows the first terminal, which the test opened");

    let db = terminal(2, "db-0");
    db.terminal.set_state(SessionState::Open);
    let view = window.state().view.clone();
    window.state_mut().view = with_terminals(view, &[web.clone(), db.clone()]);
    window.run();
    assert!(shows(&window, &title(&db)), "a tab for the new terminal");
    // The dock shows the new terminal: its grid paints with no panic over the fonts.
    assert!(window.query_by_label("Terminal").is_some(), "the new terminal paints in the dock");
}

/// The app has no sound for a bell: the grid and the tab of its terminal flash for a short
/// time, also the tab of a terminal that the dock does not show.
#[test]
fn a_bell_flashes_the_grid_and_the_tab_for_a_short_time() {
    let (web, db) = (terminal(1, "web-1"), terminal(2, "db-0"));
    let mut window = opened(&[web.clone(), db.clone()]);
    let rings = |window: &Window, label: &str| {
        window.query_all_by_label(label).any(|node| node.accesskit_node().description().as_deref() == Some("bell"))
    };
    web.terminal.feed(b"\x07");
    until(&mut window, "the flash of the grid and of the tab", |window| {
        rings(window, "Terminal") && rings(window, &title(&web))
    });
    assert!(!rings(&window, &title(&db)), "the other terminal rang no bell");
    until(&mut window, "the end of the flash", |window| !rings(window, "Terminal") && !rings(window, &title(&web)));

    db.terminal.feed(b"\x07");
    until(&mut window, "the flash of a tab that does not show", |window| rings(window, &title(&db)));
}
