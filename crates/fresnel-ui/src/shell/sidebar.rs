//! The sidebar: the views of the center of the window. With the tables of the other
//! kinds, it gets the groups and the kinds of plan section 11.3.

use egui::{Align2, Sense, TextStyle, Ui, WidgetInfo, WidgetType};

use crate::theme::{self, Palette, ROW_HEIGHT};

pub(super) const WIDTH: f32 = 148.0;

/// What the center of the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    Pods,
    Connections,
}

impl Page {
    const ALL: [Self; 2] = [Self::Pods, Self::Connections];

    fn title(self) -> &'static str {
        match self {
            Self::Pods => "Pods",
            Self::Connections => "Connections",
        }
    }
}

/// Draws the entries. A click on an entry opens its page.
pub(super) fn show(ui: &mut Ui, open: &mut Page) {
    let palette = theme::palette(ui);
    ui.add_space(8.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    for page in Page::ALL {
        if entry(ui, palette, page.title(), *open == page).clicked() {
            *open = page;
        }
    }
}

/// One entry. The entries are one choice of many, so a screen reader gets a radio button.
fn entry(ui: &mut Ui, palette: &Palette, title: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, selected, title));
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 4.0, palette.accent_bg);
    } else if response.hovered() {
        painter.rect_filled(rect, 4.0, palette.bg_raised);
    }
    let color = if selected { palette.text } else { palette.text_muted };
    let left = rect.left_center() + egui::vec2(10.0, 0.0);
    painter.text(left, Align2::LEFT_CENTER, title, TextStyle::Body.resolve(ui.style()), color);
    response
}
