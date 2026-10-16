//! The close of the window while a forward has its local port (plan section 11.6): the app
//! asks first, since the local ports of the forwards close with it. The dialog offers minimize, quit and
//! cancel. Cmd+Q ends the app with no question: eframe gives the app no way to refuse it.

use egui::{Context, Id, Key, Modal, RichText, ViewportCommand};
use fresnel_core::app::CoreView;

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

    /// Stops a close of the window while a forward runs, and asks the user.
    pub(crate) fn show(&mut self, ctx: &Context, view: &CoreView) {
        // A forward whose listener is open: a forward that the server refuses keeps its port.
        let running = view.forwards.iter().filter(|row| !row.view.addresses.is_empty()).count();
        if ctx.input(|input| input.viewport().close_requested()) && running > 0 && !self.allowed {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.asking = true;
        }
        if !self.asking {
            return;
        }
        let modal = Modal::new(Id::new("quit with forwards")).show(ctx, |ui| {
            let palette = theme::palette(ui);
            ui.label(RichText::new("Quit Fresnel?").heading());
            let forwards = format::count(running as u64, "forward");
            let text = format!("{forwards} of the app run: a quit closes their local ports.");
            ui.label(RichText::new(text).color(palette.text_muted));
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
