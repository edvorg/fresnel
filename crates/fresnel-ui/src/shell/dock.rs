//! The dock: the tabs below the center of the window. When the user can move a tab to
//! some other place, this module wraps `egui_tiles`, and no other module changes.

use egui::{Button, Frame, Margin, RichText, ScrollArea, Ui};
use fresnel_core::cluster::ClusterId;

use crate::theme::{self, BAR_HEIGHT, ClusterColors};
use crate::widgets::tone::Tone;
use crate::widgets::{self, dot};

/// The height of the dock at the first start. The user changes it with the pointer.
pub(super) const HEIGHT: f32 = 170.0;
/// The height of the dock with a log view, an editor or a terminal at the first start: they
/// need rows, and their tabs have a bar. The dock with one of them keeps a height of its own.
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
    /// An editor: the identity of its tab.
    Editor(u64),
    /// A terminal of the core: its identity.
    Terminal(u64),
}

/// The title of the tab of a log view, an editor or a terminal in the tab bar.
#[derive(Clone, Debug)]
pub(crate) struct Title {
    pub(crate) tab: Tab,
    pub(crate) text: String,
    pub(crate) cluster: ClusterId,
    /// The state of its view or of its session in one tone.
    pub(crate) tone: Tone,
}

impl Tab {
    const FIXED: [(Self, &'static str); 3] =
        [(Self::Operations, "Operations"), (Self::Journal, "Journal"), (Self::Connections, "Connections")];
}

/// Draws the tab bar. A click on a title opens its tab. Each log view, editor and terminal has a
/// tab with the color of its cluster, its title in the tone of its state and a close button.
/// Gives the tab that the user closed.
///
/// Eight log tabs can be wider than the window, so the bar scrolls, and a tab that opens
/// comes into it once: in each frame, it would hold the bar.
/// `focus` gives the focus of the keys to the tab that is open.
pub(super) fn tab_bar(
    ui: &mut Ui,
    (open, focus): (&mut Tab, bool),
    titles: &[Title],
    colors: &ClusterColors,
) -> Option<Tab> {
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
                for title in titles {
                    ui.add_space(4.0);
                    dot(ui, colors.color(&title.cluster, palette));
                    let text = match title.tone {
                        Tone::Warn | Tone::Error => RichText::new(&title.text).color(title.tone.color(palette)),
                        _ => RichText::new(&title.text),
                    };
                    let tab = ui.selectable_value(open, title.tab, text);
                    if focus && *open == title.tab {
                        tab.request_focus();
                    }
                    let close = ui.add(Button::new(RichText::new("×").color(palette.text_muted)).frame(false).small());
                    widgets::name(&close, &format!("Close {}", title.text));
                    if opened && *open == title.tab {
                        tab.union(close.clone()).scroll_to_me(None);
                    }
                    if close.clicked() {
                        closed = Some(title.tab);
                    }
                }
            });
        });
    });
    closed
}
