//! The key of each command (`[shortcuts]`): the text of the key, which the reader of the
//! file reads, and an empty text for a command with no key.

use egui::{Grid, RichText, Ui};
use fresnel_core::commands::{COMMAND_KEY, Keys, Shortcut, TABLE};
use fresnel_core::config::EditValue;

use super::draft::{Draft, path};
use crate::theme::{Palette, SMALL};

pub(super) fn show(ui: &mut Ui, palette: &Palette, keys: &Keys, draft: &mut Draft) {
    let note = format!(
        "A key such as {COMMAND_KEY}+Shift+K, Alt+Left or F5. An empty field gives the command no key; a key \
         of an other command needs an empty field there in the same save."
    );
    ui.label(RichText::new(note).size(SMALL).color(palette.text_muted));
    Grid::new("settings keys").num_columns(3).spacing([16.0, 4.0]).show(ui, |ui| {
        for entry in TABLE {
            let key = path(&["shortcuts", entry.name]);
            let now = keys.of(entry.id).map(|shortcut| shortcut.to_string()).unwrap_or_default();
            ui.label(entry.label).on_hover_text(entry.name);
            draft.field(ui, palette, (&key, &format!("Key of {}", entry.label), false), &now, |text| {
                match text.trim() {
                    "" => Ok(EditValue::Text(String::new())),
                    text => Shortcut::parse(text).map(|shortcut| EditValue::Text(shortcut.to_string())),
                }
            });
            draft.reset(ui, (&key, entry.label), entry.key.to_string() != now);
            ui.end_row();
        }
    });
}
