//! The question before a quit (plan section 11.6): a quit while a terminal or a forward runs
//! asks first, since it ends the programs of the terminals in their pods and closes the local
//! ports of the forwards. The dialog offers minimize, quit and cancel; the setting
//! `ask_before_quit` turns it off.
//!
//! Each quit takes one decision, [`QuitQuestion::decide`]: a close of the window (its button,
//! Alt+F4 and the taskbar of Windows, the quit key of egui) in the logic of a frame, and a quit
//! of the app (the Quit item of macOS, Cmd+Q, the Dock) in the answer of the program to the
//! system, outside a frame. eframe runs no ui pass for a hidden window, only `App::logic`, so
//! the stop of a close and the return of the window are steps of the logic, and the dialog a
//! step of the frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Context, Id, Key, Modal, RichText, ViewportCommand};
use fresnel_core::app::CoreView;
use fresnel_core::config::table::Setting;

use crate::format;
use crate::theme;

/// What a quit does now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quit {
    /// Nothing that a quit ends runs, the setting is off, or the user chose "Quit".
    Now,
    /// The question is open: the quit waits for the choice of the user.
    Ask,
}

/// The question before a quit, open or closed. A copy shares the state: the program keeps
/// one for a quit of the app, which the system asks for outside a frame.
#[derive(Clone, Debug, Default)]
pub struct QuitQuestion(Arc<State>);

/// Flags, not a lock: a quit of the app comes from the system on the thread of the window,
/// between two frames, and needs no wait.
#[derive(Debug, Default)]
struct State {
    asking: AtomicBool,
    /// The user chose "Quit": each quit after it goes through.
    allowed: AtomicBool,
    /// A question opened, and the next logic brings the window back to show it.
    back: AtomicBool,
}

/// What the user chose.
enum Choice {
    Minimize,
    Quit,
    Cancel,
}

impl QuitQuestion {
    /// The one decision of each quit. `Now` while no terminal and no forward would end,
    /// with the setting off or after the choice "Quit"; else the question opens, and the next
    /// logic brings the window back, also from the Dock or a minimize. `view` copies the
    /// state of the core, and only a quit calls it.
    pub fn decide(&self, ctx: &Context, view: impl FnOnce() -> CoreView) -> Quit {
        if self.0.allowed.load(Ordering::Relaxed) {
            return Quit::Now;
        }
        let view = view();
        if !view.config.flag(Setting::AskQuit) || ends(&view).is_empty() {
            return Quit::Now;
        }
        self.0.asking.store(true, Ordering::Relaxed);
        self.0.back.store(true, Ordering::Relaxed);
        // A quit of the app comes between two frames, and eframe runs the logic of a
        // minimized window only after a request.
        ctx.request_repaint();
        Quit::Ask
    }

    pub(crate) fn is_open(&self) -> bool {
        self.0.asking.load(Ordering::Relaxed)
    }

    /// The logic of each frame, also of a hidden window: a close of the window that has to
    /// ask stops, and the window of a new question comes back and takes the focus. A close of
    /// a minimized window, as from the taskbar of Windows, ended the app with no question in
    /// a frame with no ui pass, and a question in a minimized window shows nowhere.
    pub(crate) fn logic(&self, ctx: &Context, view: impl FnOnce() -> CoreView) {
        if ctx.input(|input| input.viewport().close_requested()) && self.decide(ctx, view) == Quit::Ask {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        }
        if self.0.back.swap(false, Ordering::Relaxed) {
            if ctx.input(|input| input.viewport().minimized == Some(true)) {
                ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            }
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
    }

    /// Draws the question while it is open.
    pub(crate) fn show(&self, ctx: &Context, view: &CoreView) {
        if !self.is_open() {
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
                self.0.asking.store(false, Ordering::Relaxed);
            }
            Some(Choice::Quit) => {
                self.0.allowed.store(true, Ordering::Relaxed);
                ctx.send_viewport_cmd(ViewportCommand::Close);
                self.0.asking.store(false, Ordering::Relaxed);
            }
            Some(Choice::Cancel) => self.0.asking.store(false, Ordering::Relaxed),
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
