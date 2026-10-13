//! The dock: the tabs of the window. When the tables of the kinds arrive, this module
//! wraps `egui_tiles`, and no other module changes.

use egui::{Frame, Margin, Ui};

use crate::theme::{self, BAR_HEIGHT};

/// One tab of the dock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Connections,
    Journal,
}

impl Tab {
    const ALL: [Self; 2] = [Self::Connections, Self::Journal];

    fn title(self) -> &'static str {
        match self {
            Self::Connections => "Connections",
            Self::Journal => "Journal",
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
            // The bar takes the full width of the window.
            ui.add_space(ui.available_width());
        });
    });
}
