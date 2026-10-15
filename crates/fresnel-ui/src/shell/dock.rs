//! The dock: the tabs below the center of the window. When the user can move a tab to
//! some other place, this module wraps `egui_tiles`, and no other module changes.

use egui::{Button, Frame, Margin, RichText, Ui};

use crate::theme::{self, BAR_HEIGHT, ClusterColors};
use crate::views::logs::Title;
use crate::widgets::tone::Tone;
use crate::widgets::{self, dot};

/// The height of the dock at the first start. The user changes it with the pointer.
pub(super) const HEIGHT: f32 = 170.0;
/// The height of the dock with a log view at the first start: a log needs rows, and its tab
/// has a bar. The dock with a log view keeps a height of its own.
pub(super) const LOG_HEIGHT: f32 = 380.0;
/// The smallest dock shows its tab bar only, so the center has the rest of the window.
pub(super) const SMALLEST: f32 = BAR_HEIGHT;

/// One tab of the dock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Operations,
    Journal,
    Connections,
    /// A log view: the identity of its tab.
    Logs(u64),
}

impl Tab {
    const FIXED: [(Self, &'static str); 3] =
        [(Self::Operations, "Operations"), (Self::Journal, "Journal"), (Self::Connections, "Connections")];
}

/// Draws the tab bar. A click on a title opens its tab. Each log view has a tab with the
/// color of its cluster, its title in the tone of its sources and a close button. Gives the
/// log tab that the user closed.
pub(super) fn tab_bar(ui: &mut Ui, open: &mut Tab, logs: &[Title], colors: &ClusterColors) -> Option<u64> {
    let palette = theme::palette(ui);
    let mut closed = None;
    Frame::new().fill(palette.bg_chrome).inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(BAR_HEIGHT - 2.0);
            for (tab, title) in Tab::FIXED {
                ui.selectable_value(open, tab, title);
            }
            for title in logs {
                ui.add_space(4.0);
                dot(ui, colors.color(&title.cluster, palette));
                let text = match title.tone {
                    Tone::Warn | Tone::Error => RichText::new(&title.text).color(title.tone.color(palette)),
                    _ => RichText::new(&title.text),
                };
                ui.selectable_value(open, Tab::Logs(title.id), text);
                let close = ui.add(Button::new(RichText::new("×").color(palette.text_muted)).frame(false).small());
                widgets::name(&close, &format!("Close {}", title.text));
                if close.clicked() {
                    closed = Some(title.id);
                }
            }
            // The bar takes the full width of the dock.
            ui.add_space(ui.available_width());
        });
    });
    closed
}
