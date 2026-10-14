//! The parts of each section of the panel: a heading, a line with a label and a value, the
//! chips of the labels, and a list of links.

use std::collections::BTreeMap;

use egui::{Align, Label, Layout, RichText, Sense, Ui, WidgetInfo, WidgetType};

use crate::format::{self, short};
use crate::theme::{self, Palette};

/// The width of the labels of the metadata.
const LABEL_WIDTH: f32 = 104.0;
/// The most characters of one value. The value of an annotation can be a full manifest,
/// and no other text of an object has a limit that each server holds.
pub(super) const VALUE_LIMIT: usize = 240;
/// The most chips of the labels or of the annotations, and the most links of a list. An
/// object can have thousands of them.
pub(super) const ITEM_LIMIT: usize = 100;
/// The text of no value.
pub(crate) const NONE: &str = "–";

pub(crate) fn heading(ui: &mut Ui, palette: &Palette, title: &str) {
    ui.label(RichText::new(title).size(11.0).color(palette.text_faint));
    ui.add_space(2.0);
}

/// One line of a section: the label at the left and the value at the right. A long value
/// wraps below itself and not below the label.
pub(crate) fn field(ui: &mut Ui, palette: &Palette, label: &str, value: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.allocate_ui_with_layout(egui::vec2(LABEL_WIDTH, 20.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_width(LABEL_WIDTH);
            ui.add(Label::new(RichText::new(label).color(palette.text_muted)).truncate());
        });
        ui.vertical(value);
    });
    ui.add_space(2.0);
}

pub(crate) fn text_field(ui: &mut Ui, palette: &Palette, label: &str, value: &str) {
    field(ui, palette, label, |ui| {
        ui.add_space(1.0);
        ui.add(Label::new(short(value, VALUE_LIMIT)).wrap());
    });
}

/// A note in the tone of a state, for a part that the panel cannot show as current.
pub(super) fn note(ui: &mut Ui, color: egui::Color32, text: &str) {
    ui.add(Label::new(RichText::new(text).color(color)).wrap());
}

/// The note of the items of a list that do not show.
pub(super) fn more(ui: &mut Ui, palette: &Palette, total: usize) {
    if let Some(more) = total.checked_sub(ITEM_LIMIT).filter(|more| *more > 0) {
        ui.label(RichText::new(format!("and {} more", format::thousands(more as u64))).color(palette.text_muted));
    }
}

/// One chip for each label or annotation, or the mark of no value. A chip has a fixed
/// size, so a line of chips breaks between two of them. The first chips show, and a note
/// has the count of the other ones: each chip is a layout in each frame.
pub(super) fn chips(ui: &mut Ui, palette: &Palette, values: Option<&BTreeMap<String, String>>) {
    let Some(values) = values.filter(|values| !values.is_empty()) else {
        ui.add_space(1.0);
        ui.label(RichText::new(NONE).color(palette.text_faint));
        return;
    };
    // A text that is longer than the panel wraps in its chip.
    let widest = ui.available_width() - 12.0;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for (key, value) in values.iter().take(ITEM_LIMIT) {
            let text = format!("{}={}", short(key, VALUE_LIMIT), short(value, VALUE_LIMIT));
            let galley = ui.painter().layout(text.clone(), theme::mono(), palette.text, widest);
            let (rect, response) = ui.allocate_exact_size(galley.size() + egui::vec2(12.0, 6.0), Sense::hover());
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &text));
            ui.painter().rect_filled(rect, 4.0, palette.bg_raised);
            ui.painter().galley(rect.min + egui::vec2(6.0, 3.0), galley, palette.text);
        }
    });
    more(ui, palette, values.len());
}
