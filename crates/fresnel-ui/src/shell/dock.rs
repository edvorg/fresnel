//! The dock: the tabs below the center of the window. When the user can move a tab to
//! some other place, this module wraps `egui_tiles`, and no other module changes.

use egui::{Button, Frame, Margin, RichText, ScrollArea, Ui};

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
///
/// Eight log tabs can be wider than the window, so the bar scrolls, and a tab that opens
/// comes into it once: in each frame, it would hold the bar.
/// `focus` gives the focus of the keys to the tab that is open.
pub(super) fn tab_bar(
    ui: &mut Ui,
    (open, focus): (&mut Tab, bool),
    logs: &[Title],
    colors: &ClusterColors,
) -> Option<u64> {
    let palette = theme::palette(ui);
    let mut closed = None;
    let last_id = ui.id().with("tab of the last frame");
    let opened = ui.data_mut(|data| {
        let last = data.get_temp::<Tab>(last_id);
        data.insert_temp(last_id, *open);
        last != Some(*open)
    });
    Frame::new().fill(palette.bg_chrome).inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
        ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(BAR_HEIGHT - 2.0);
                for (tab, title) in Tab::FIXED {
                    let button = ui.selectable_value(open, tab, title);
                    if focus && *open == tab {
                        button.request_focus();
                    }
                }
                for title in logs {
                    ui.add_space(4.0);
                    dot(ui, colors.color(&title.cluster, palette));
                    let text = match title.tone {
                        Tone::Warn | Tone::Error => RichText::new(&title.text).color(title.tone.color(palette)),
                        _ => RichText::new(&title.text),
                    };
                    let tab = ui.selectable_value(open, Tab::Logs(title.id), text);
                    if focus && *open == Tab::Logs(title.id) {
                        tab.request_focus();
                    }
                    let close = ui.add(Button::new(RichText::new("×").color(palette.text_muted)).frame(false).small());
                    widgets::name(&close, &format!("Close {}", title.text));
                    if opened && *open == Tab::Logs(title.id) {
                        tab.union(close.clone()).scroll_to_me(None);
                    }
                    if close.clicked() {
                        closed = Some(title.id);
                    }
                }
            });
        });
    });
    closed
}
