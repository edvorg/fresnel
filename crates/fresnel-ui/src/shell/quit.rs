//! The question before a quit (plan section 11.6): a close of the window or Cmd+Q while a
//! terminal or a forward runs asks first, since a quit ends the programs of the terminals in
//! their pods and closes the local ports of the forwards. The dialog offers minimize, quit and
//! cancel; the setting `ask_before_quit` turns it off.
//!
//! Each way to quit is a close of the window, which the app can refuse: egui makes Cmd+Q
//! (Ctrl+Q on Windows) one, and the program makes the Quit item of the macOS menu one too.
//! eframe runs no ui pass for a hidden window, only `App::logic`, so the stop of a close is
//! a step of the logic ([`QuitGuard::stop_close`]) and the dialog a step of the frame.

use egui::{Context, Id, Key, Modal, RichText, ViewportCommand};
use fresnel_core::app::CoreView;
use fresnel_core::config::table::Setting;

use crate::format;
use crate::theme;

/// The question before a close, open or closed.
#[derive(Debug, Default)]
pub(crate) struct QuitGuard {
    asking: bool,
    /// The user chose "Quit": the next close goes through.
    allowed: bool,
}

/// What the user chose.
enum Choice {
    Minimize,
    Quit,
    Cancel,
}

impl QuitGuard {
    pub(crate) fn is_open(&self) -> bool {
        self.asking
    }

    /// Stops a close of the window while a terminal or a forward runs, and opens the question.
    /// It runs in the logic of each frame, also of a hidden window: a close of a minimized
    /// window, as from the taskbar of Windows, ended the app with no question in a frame with
    /// no ui pass. Such a window comes back first, as its question would show nowhere. `view`
    /// copies the state of the core, and only a close calls it.
    pub(crate) fn stop_close(&mut self, ctx: &Context, view: impl FnOnce() -> CoreView) {
        if self.allowed || !ctx.input(|input| input.viewport().close_requested()) {
            return;
        }
        let view = view();
        if !view.config.flag(Setting::AskQuit) || ends(&view).is_empty() {
            return;
        }
        ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        if ctx.input(|input| input.viewport().minimized == Some(true)) {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        self.asking = true;
    }

    /// Draws the question while it is open.
    pub(crate) fn show(&mut self, ctx: &Context, view: &CoreView) {
        if !self.asking {
            return;
        }
        let ends = ends(view);
        let modal = Modal::new(Id::new("quit")).show(ctx, |ui| {
            let palette = theme::palette(ui);
            ui.label(RichText::new("Quit Fresnel?").heading());
            for text in &ends {
                ui.label(RichText::new(text).color(palette.text_muted));
            }
            ui.add_space(8.0);
            let mut choice = None;
            ui.horizontal(|ui| {
                if ui.button("Minimize").clicked() {
                    choice = Some(Choice::Minimize);
                }
                if ui.button("Quit").clicked() {
                    choice = Some(Choice::Quit);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
            choice
        });
        let escape = ctx.input(|input| input.key_pressed(Key::Escape)).then_some(Choice::Cancel);
        match modal.inner.or(escape) {
            Some(Choice::Minimize) => {
                ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                self.asking = false;
            }
            Some(Choice::Quit) => {
                self.allowed = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
                self.asking = false;
            }
            Some(Choice::Cancel) => self.asking = false,
            None => {}
        }
    }
}

/// What a quit ends, a text for each part: the terminals whose program can still run, and the
/// forwards whose listener is open (a forward that the server refuses keeps its port).
fn ends(view: &CoreView) -> Vec<String> {
    let terminals = view.terminals.iter().filter(|terminal| !terminal.terminal.state().is_final()).count();
    let forwards = view.forwards.iter().filter(|row| !row.view.addresses.is_empty()).count();
    let mut ends = Vec::new();
    let (terminals_text, forwards_text) =
        (format::count(terminals as u64, "terminal"), format::count(forwards as u64, "forward"));
    match terminals {
        0 => {}
        1 => ends.push("1 terminal is open: a quit ends its program in the pod.".to_owned()),
        _ => ends.push(format!("{terminals_text} are open: a quit ends their programs in the pods.")),
    }
    match forwards {
        0 => {}
        1 => ends.push("1 forward of the app runs: a quit closes its local port.".to_owned()),
        _ => ends.push(format!("{forwards_text} of the app run: a quit closes their local ports.")),
    }
    ends
}
