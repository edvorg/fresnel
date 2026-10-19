//! The overlay of the keys (plan section 11.5): each command of the command table with its
//! key after the settings, and the keys of the table, the sidebar, the dock and an editor.
//! `?` opens it, and Escape or `?` closes it.

use egui::{Context, Grid, Id, Modal, RichText, ScrollArea, Ui};
use fresnel_core::commands::{COMMAND_KEY, CommandId, Keys, TABLE};

use crate::shell::keys;
use crate::theme;

/// The width of the column of the keys, so the three lists align.
const KEY_WIDTH: f32 = 220.0;

/// The keys of the areas, which the command table does not have. `Cmd` stands for the name of
/// the command key of the system.
const AREA_KEYS: &[(&str, &str)] = &[
    ("Up, Down, Page Up, Page Down, Home, End", "Move the cursor of a table; Shift selects a range"),
    ("Cmd+A, Escape", "Select each row of a table, or no row"),
    ("Up, Down, Left, Right", "Move the focus in the sidebar, the tabs of the panel and the dock"),
    ("Space, Enter", "Open the entry or the tab of the focus"),
    ("Tab, Shift+Tab", "Move the focus to the next field or button, or the one before"),
    ("Cmd+F, Cmd+G, Shift+Cmd+G", "Find in an editor, and go to the next hit or the one before"),
    ("F8, Shift+F8", "Go to the next mark of the check of an editor, or the one before"),
    ("Cmd+Z, Shift+Cmd+Z", "Undo and redo in an editor, where Tab indents and Shift+Tab unindents"),
];

/// Draws the overlay while `open` is true. Escape and a click beside it close it.
pub(crate) fn show(ctx: &Context, keys: &Keys, open: &mut bool) {
    if !*open {
        return;
    }
    // `?` closes the overlay that it opened. A shortcut with no Shift takes `?` with Shift.
    if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Questionmark)) {
        *open = false;
        return;
    }
    let modal = Modal::new(Id::new("keys overlay")).show(ctx, |ui| {
        ui.set_width(640.0);
        ui.label(RichText::new("Keys").heading().strong());
        ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
            section(ui, "The app", keys, |id| !matches!(id, CommandId::Object(_)));
            section(ui, "The object of the panel", keys, |id| matches!(id, CommandId::Object(_)));
            title(ui, "The areas");
            Grid::new("keys of the areas").num_columns(2).min_col_width(KEY_WIDTH).spacing([24.0, 4.0]).show(
                ui,
                |ui| {
                    for (keys, what) in AREA_KEYS {
                        ui.label(RichText::new(keys.replace("Cmd", COMMAND_KEY)).monospace());
                        ui.label(*what);
                        ui.end_row();
                    }
                },
            );
        });
        ui.add_space(6.0);
        ui.button("Close").clicked()
    });
    if modal.inner || modal.should_close() {
        *open = false;
    }
}

fn title(ui: &mut Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).strong().color(theme::palette(ui).text_muted));
}

/// The commands of a part of the table that have a key now.
fn section(ui: &mut Ui, text: &str, keys: &Keys, part: impl Fn(CommandId) -> bool) {
    title(ui, text);
    Grid::new(text).num_columns(2).min_col_width(KEY_WIDTH).spacing([24.0, 4.0]).show(ui, |ui| {
        for entry in TABLE.iter().filter(|entry| part(entry.id)) {
            let Some(key) = keys::text(ui.ctx(), keys, entry.id) else {
                continue;
            };
            ui.label(RichText::new(key).monospace());
            ui.label(entry.label);
            ui.end_row();
        }
    });
}
