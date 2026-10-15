//! The changes of the settings view that the file does not have yet.
//!
//! A control shows the value of the file, or of the draft after a change. A text field keeps
//! the text of the user while it does not fit, with the reason beside it, so a number that
//! the user types digit by digit never jumps. A change back to the value of the file is no
//! change. Save sends one `Command::EditSettings` with each change.

use std::collections::BTreeMap;

use egui::{Button, RichText, TextEdit, Ui};
use fresnel_core::config::{Edit, EditValue};

use crate::theme::{Palette, SMALL};
use crate::widgets;

/// The path of a key in the file: `["network", "keepalive_secs"]`.
pub(super) type Path = Vec<String>;

/// The width of a field of a number or a short text.
const FIELD_WIDTH: f32 = 220.0;

#[derive(Debug, Default)]
pub(super) struct Draft {
    /// The value of each key that the user changed.
    values: BTreeMap<Path, EditValue>,
    /// The text of each field that the user changed, also while it does not fit.
    texts: BTreeMap<Path, String>,
    /// Why the text of a field does not fit.
    errors: BTreeMap<Path, String>,
}

impl Draft {
    /// The changes for the file.
    pub(super) fn edits(&self) -> Vec<Edit> {
        self.values.iter().map(|(path, value)| Edit { path: path.clone(), value: value.clone() }).collect()
    }

    pub(super) fn count(&self) -> usize {
        self.values.len()
    }

    /// Why the draft cannot go to the file, if a field does not fit.
    pub(super) fn problem(&self) -> Option<String> {
        let count = self.errors.len();
        (count > 0).then(|| crate::format::count(count as u64, "field") + " to correct first")
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    /// The value of a control: the draft after a change, or `current`.
    pub(super) fn get<'a>(&'a self, path: &Path) -> Option<&'a EditValue> {
        self.values.get(path)
    }

    /// Sets the value of a control. The value of the file is no change.
    pub(super) fn set(&mut self, path: &Path, value: EditValue, current: &EditValue) {
        self.texts.remove(path);
        self.errors.remove(path);
        if value == *current {
            self.values.remove(path);
        } else {
            self.values.insert(path.clone(), value);
        }
    }

    /// A text field, of one line or of one entry for each line. `current` is the text of the
    /// value of the file, and `read` gives the value of a text or why it does not fit.
    pub(super) fn field(
        &mut self,
        ui: &mut Ui,
        palette: &Palette,
        (path, name, multiline): (&Path, &str, bool),
        current: &str,
        read: impl Fn(&str) -> Result<EditValue, String>,
    ) {
        let mut text = self.texts.get(path).cloned().unwrap_or_else(|| match self.values.get(path) {
            Some(EditValue::Remove) => String::new(),
            _ => current.to_owned(),
        });
        let edit =
            if multiline { TextEdit::multiline(&mut text).desired_rows(2) } else { TextEdit::singleline(&mut text) };
        let field = ui.add(edit.desired_width(FIELD_WIDTH));
        widgets::name(&field, name);
        if field.changed() {
            self.errors.remove(path);
            if text == current {
                self.values.remove(path);
                self.texts.remove(path);
            } else {
                match read(&text) {
                    Ok(value) => {
                        self.values.insert(path.clone(), value);
                    }
                    Err(error) => {
                        self.values.remove(path);
                        self.errors.insert(path.clone(), error);
                    }
                }
                self.texts.insert(path.clone(), text);
            }
        }
        if let Some(error) = self.errors.get(path) {
            ui.label(RichText::new(error).size(SMALL).color(palette.error));
        }
    }

    /// The button that gives a key of the file its default: the key goes from the file.
    pub(super) fn reset(&mut self, ui: &mut Ui, (path, name): (&Path, &str), in_file: bool) {
        if !in_file || self.values.get(path) == Some(&EditValue::Remove) {
            return;
        }
        let button = ui.add(Button::new(RichText::new("Default").size(SMALL)).small());
        widgets::name(&button, &format!("Default of {name}"));
        if button.on_hover_text("The key goes from the file, and the setting gets its default.").clicked() {
            self.texts.remove(path);
            self.errors.remove(path);
            self.values.insert(path.clone(), EditValue::Remove);
        }
    }
}

/// The path of a key as a vector.
pub(super) fn path(parts: &[&str]) -> Path {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

/// The lines of a list field as a list: one entry for each line that is not empty.
pub(super) fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect()
}
