//! The dialog of an action: a modal window over the page, so no other input reaches the
//! page while it asks. Escape and a click beside it close it with no action.

use egui::{Id, Modal, RichText, Ui};
use fresnel_core::ops::{Order, Target};

use fresnel_core::terminal::TerminalRequest;

use super::{Commands, ConfirmDialog, DrainDialog, Found, TerminalConfirm};
use crate::shell::Scene;
use crate::theme::Palette;

/// The width of the dialog.
const WIDTH: f32 = 620.0;

/// One open dialog.
pub(crate) enum Dialog {
    /// Asks before a change: the targets, the input of the action and its warnings.
    Confirm(ConfirmDialog),
    /// Asks before a drain: its options and each pod of the node with its verdict.
    Drain(DrainDialog),
    /// The `kubectl` commands of an object, each one with a copy button.
    Commands(Commands),
    /// Asks before a terminal that changes the cluster: a debug container or a node shell.
    Terminal(TerminalConfirm),
}

/// How a dialog ended in a frame.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Close {
    /// It stays open.
    Open,
    /// The user closed it with no action.
    Cancelled,
    /// The user confirmed: one order for each object, and the note for the page.
    Run { orders: Vec<Order>, note: String },
    /// The user copied this command.
    Copied(String),
    /// The user confirmed a terminal that changes the cluster.
    Terminal(TerminalRequest),
}

impl Dialog {
    /// Draws the dialog. `look` finds an object of an action in the store of its cluster.
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        scene: &mut Scene<'_>,
        look: &dyn Fn(&Target) -> Found,
    ) -> Close {
        let modal = Modal::new(Id::new("action dialog")).show(ctx, |ui| {
            ui.set_width(WIDTH);
            match self {
                Self::Confirm(confirm) => confirm.show(ui, scene, look),
                Self::Drain(drain) => drain.show(ui, scene, look),
                Self::Commands(commands) => commands.show(ui),
                Self::Terminal(terminal) => terminal.show(ui, scene),
            }
        });
        match modal.inner {
            Close::Open if modal.should_close() => Close::Cancelled,
            close => close,
        }
    }
}

/// The title of a dialog.
pub(super) fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(16.0).strong());
    ui.add_space(6.0);
}

/// One warning line.
pub(super) fn warning(ui: &mut Ui, palette: &Palette, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(palette.warn)).wrap());
}
