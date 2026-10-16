//! The terminals of the dock on the fake core (M10): the tab and the line of its state, the
//! keys, the paste, the pointer, the size, the recorded output of busybox `vi` and `top`,
//! PNG files, and the time of a frame.
//!
//! A test gives the fake core terminals with no session: it feeds the output of a remote
//! program itself, and reads the input that the session would send.
//!
//! - `tab`: the tabs and the line of the state. `input`: the keys, the paste and the
//!   pointer. `screen`: the size, the history and the recorded programs.
//! - `snapshots`: PNG files. `frame_time`: 10 MB of output and a full redraw of the screen.

mod frame_time;
mod input;
mod open;
mod screen;
mod snapshots;
mod tab;

use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::CoreView;
use fresnel_core::cluster::ClusterId;
use fresnel_core::store::ObjectKey;
use fresnel_core::terminal::{GridSize, Options, Program, Terminal, TerminalRequest, TerminalView};

use crate::data::{live, pods};
use crate::fake::{Window, open_tab, view, window};

macro_rules! recording {
    ($name:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../fresnel-core/fixtures/terminal/", $name))
    };
}

/// The output of busybox 1.36.1 in a TTY of 80 x 24 (`fresnel-core/fixtures/terminal`).
pub(crate) const VI: &[u8] = recording!("vi.bin");
pub(crate) const TOP: &[u8] = recording!("top.bin");
pub(crate) const SHELL: &[u8] = recording!("shell.bin");

/// A terminal of a shell in the container `app` of the pod `name` of `ams-prod`.
pub(crate) fn terminal(id: u64, name: &str) -> TerminalView {
    let request = TerminalRequest {
        cluster: ClusterId::new("ams-prod"),
        object: ObjectKey::new("shop", name),
        uid: format!("uid-{name}").into(),
        container: "app".into(),
        program: Program::Shell,
    };
    let options = Options { size: GridSize::default(), scrollback: 1000 };
    TerminalView { id, request, terminal: Terminal::new(options) }
}

/// The view of the fake core with these terminals.
pub(crate) fn with_terminals(mut view: CoreView, terminals: &[TerminalView]) -> CoreView {
    view.terminals = Arc::from(terminals);
    view
}

/// The window of one cluster with these terminals, and the tab of the first one open.
pub(crate) fn opened(terminals: &[TerminalView]) -> Window {
    let mut window = window(with_terminals(view(vec![live("ams-prod", &pods())]), terminals));
    if let Some(first) = terminals.first() {
        open_tab(&mut window, &title(first));
    }
    window
}

/// The title of the tab of a terminal.
pub(crate) fn title(terminal: &TerminalView) -> String {
    format!("Shell {} · app", terminal.request.object.name)
}

/// The text of each row of the terminal on the screen, from the top.
pub(crate) fn rows(window: &Window) -> Vec<String> {
    let area = window.get_by_label("Terminal").rect();
    let mut rows: Vec<(f32, String)> = window
        .query_all_by_role(Role::ListItem)
        .filter(|node| area.contains(node.rect().center()))
        .map(|node| (node.rect().top(), node.accesskit_node().label().unwrap_or_default().to_owned()))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, text)| text).collect()
}

/// The input that waits for the session, each piece.
pub(crate) fn input(terminal: &Terminal) -> Vec<u8> {
    std::iter::from_fn(|| terminal.take_input()).flatten().collect()
}

/// Clicks into the terminal: it takes the focus of the keys.
pub(crate) fn focus(window: &mut Window) {
    window.get_by_label("Terminal").click();
    window.run();
}
