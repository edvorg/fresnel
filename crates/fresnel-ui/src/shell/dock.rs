//! The dock: the tabs below the center of the window. When the user can move a tab to
//! some other place, this module wraps `egui_tiles`, and no other module changes.

use egui::{Frame, Margin, Ui};

use crate::theme::{self, BAR_HEIGHT};

/// The height of the dock at the first start. The user changes it with the pointer.
pub(super) const HEIGHT: f32 = 170.0;
/// The smallest dock shows its tab bar only, so the center has the rest of the window.
pub(super) const SMALLEST: f32 = BAR_HEIGHT;

/// One tab of the dock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Operations,
    Journal,
    Connections,
}

impl Tab {
    const ALL: [Self; 3] = [Self::Operations, Self::Journal, Self::Connections];

    fn title(self) -> &'static str {
        match self {
            Self::Operations => "Operations",
            Self::Journal => "Journal",
            Self::Connections => "Connections",
        }
    }
}

/// Draws the tab bar. A click on a title opens its tab.
pub(super) fn tab_bar(ui: &mut Ui, open: &mut Tab) {
    let palette = theme::palette(ui);
    Frame::new().fill(palette.bg_chrome).inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(BAR_HEIGHT - 2.0);
            for tab in Tab::ALL {
                ui.selectable_value(open, tab, tab.title());
            }
            // The bar takes the full width of the dock.
            ui.add_space(ui.available_width());
        });
    });
}
