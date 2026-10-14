//! The row that adds a key: its name and its first value. The draft checks the name with
//! the rule of the server, and the row shows why it refuses one.

use egui::{RichText, TextEdit, Ui};

use crate::theme::Palette;
use crate::widgets;

/// The text of the row, until the draft takes it.
#[derive(Default)]
pub(super) struct AddKey {
    name: String,
    value: String,
    /// Why the draft refused the last name.
    problem: Option<String>,
    /// True when the name field takes the focus in the next frame.
    focus: bool,
}

impl AddKey {
    /// Puts the focus into the name field in the next frame: the action "Add key".
    pub(super) fn focus(&mut self) {
        self.focus = true;
    }

    /// Shows why the draft refused the key.
    pub(super) fn refused(&mut self, problem: String) {
        self.problem = Some(problem);
    }

    /// Empties the row after the draft took its key.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Draws the row. Gives the name and the value of a click on "Add".
    pub(super) fn show(&mut self, ui: &mut Ui, palette: &Palette, editable: bool) -> Option<(String, Vec<u8>)> {
        let mut added = None;
        ui.add_enabled_ui(editable, |ui| {
            ui.horizontal(|ui| {
                let name = TextEdit::singleline(&mut self.name).hint_text("New key").desired_width(220.0);
                let name = ui.add(name.font(egui::TextStyle::Monospace));
                widgets::name(&name, "Name of the new key");
                if std::mem::take(&mut self.focus) {
                    name.request_focus();
                }
                if name.changed() {
                    self.problem = None;
                }
                let add = ui.add_enabled(!self.name.is_empty(), egui::Button::new("Add").small());
                widgets::name(&add, "Add the key");
                if add.clicked() {
                    added = Some((self.name.clone(), self.value.as_bytes().to_vec()));
                }
            });
            let value = TextEdit::multiline(&mut self.value).hint_text("Its value").code_editor().desired_rows(2);
            let value = ui.add(value.desired_width(f32::INFINITY));
            widgets::name(&value, "Value of the new key");
        });
        if let Some(problem) = &self.problem {
            ui.label(RichText::new(problem).color(palette.error));
        }
        added
    }
}
