//! One row for each key of the draft: its name, its state, its marks, its value and its
//! buttons (plan section 12.2).
//!
//! A value of a Secret is hidden until the user shows it: a hidden value shows dots and no
//! editor, as in Headlamp. A text value has an editor of many lines; a binary value shows
//! its size, its type and its first bytes, because a text editor would change its bytes
//! (research 08 section 8.2). A text longer than the limit of the editor shows its first
//! lines, read-only: the egui editor lays out its whole text for each key press.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Align, Label, Layout, RichText, TextEdit, Ui};
use fresnel_core::keys::{DraftKey, KeyState, Place};

use super::text;
use crate::format::short;
use crate::theme::{Palette, SMALL};
use crate::widgets;
use crate::widgets::tone::Tone;

/// The most bytes of a text that the editor takes.
const EDIT_BYTES: usize = 256 * 1024;
/// The most lines of a text that the editor takes: about 4 ms for each key press.
const EDIT_LINES: usize = 4000;
/// The lines of a long text that show.
const PREVIEW_LINES: usize = 40;
/// The most rows that an editor has before it scrolls with the panel.
const EDITOR_ROWS: usize = 12;
/// The width of the buttons of a row.
const BUTTONS_WIDTH: f32 = 220.0;
/// What a hidden value shows.
const HIDDEN: &str = "••••••••";

/// The state of the editors: the text of each editor that showed, and the keys whose value
/// the user shows.
#[derive(Default)]
pub(super) struct Editors {
    texts: BTreeMap<String, String>,
    shown: BTreeSet<String>,
}

impl Editors {
    pub(super) fn is_shown(&self, key: &str) -> bool {
        self.shown.contains(key)
    }

    pub(super) fn show(&mut self, key: &str, shown: bool) {
        if shown {
            self.shown.insert(key.to_owned());
        } else {
            self.shown.remove(key);
        }
    }

    /// The text of the editor of a key, after an edit.
    pub(super) fn text(&self, key: &str) -> Option<&str> {
        self.texts.get(key).map(String::as_str)
    }

    /// Drops the text of each editor whose key `keep` refuses: its value changed by some
    /// other way than its editor, and the next frame makes its text from the draft again.
    pub(super) fn forget(&mut self, keep: impl Fn(&str) -> bool) {
        self.texts.retain(|key, _| keep(key));
    }

    /// The text of the editor of a key, made from the value at its first use.
    fn buffer(&mut self, key: &str, value: &str) -> &mut String {
        self.texts.entry(key.to_owned()).or_insert_with(|| value.to_owned())
    }
}

/// What the user did in a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Act {
    /// The editor of the key has a new text.
    Edited(String),
    Remove(String),
    Undo(String),
    /// "Copy" of a value: the text for the note, which names no byte of it.
    Copied(String),
}

/// What all rows share.
pub(super) struct Rows<'a> {
    pub(super) palette: &'a Palette,
    /// False for an immutable object, and for a user who cannot change it.
    pub(super) editable: bool,
    /// True for a Secret: its values are hidden until the user shows them.
    pub(super) secret: bool,
}

impl Rows<'_> {
    /// Draws the row of one key. `place` is the map of the key in the read, `None` for a
    /// new key. Gives what the user did.
    pub(super) fn show(
        &self,
        ui: &mut Ui,
        editors: &mut Editors,
        key: &DraftKey<'_>,
        place: Option<Place>,
    ) -> Option<Act> {
        let shown = !self.secret || editors.is_shown(key.name);
        let mut act = None;
        ui.push_id(("key", key.name), |ui| {
            ui.horizontal(|ui| {
                let name = RichText::new(short(key.name, 120)).monospace().strong().color(self.palette.text);
                let name = if key.state == KeyState::Removed { name.strikethrough() } else { name };
                // A long name leaves the room of the buttons.
                let room = (ui.available_width() - BUTTONS_WIDTH).max(60.0);
                ui.scope(|ui| {
                    ui.set_max_width(room);
                    ui.add(Label::new(name).truncate());
                });
                if let Some((tone, word)) = state(key.state) {
                    ui.label(RichText::new(word).size(SMALL).color(tone.color(self.palette)));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    act = self.buttons(ui, editors, key, shown);
                });
            });
            // The marks have a line of their own: next to the buttons, a long mark would
            // run below them.
            let mut marks = text::marks(key.value);
            if place == Some(Place::BinaryData) {
                marks.insert(0, (Tone::Muted, "binaryData".to_owned()));
            }
            if !marks.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for (tone, mark) in marks {
                        ui.label(RichText::new(mark).size(SMALL).color(tone.color(self.palette)));
                    }
                });
            }
            if key.state != KeyState::Removed
                && let Some(edited) = self.value(ui, editors, key, shown)
            {
                act = Some(edited);
            }
        });
        ui.add_space(6.0);
        act
    }

    /// The buttons at the right of a row, from the right: remove or undo, copy, show.
    fn buttons(&self, ui: &mut Ui, editors: &mut Editors, key: &DraftKey<'_>, shown: bool) -> Option<Act> {
        let name = key.name;
        let mut act = None;
        let changed = key.state != KeyState::Same;
        let (label, what) = if changed { ("Undo", "Undo the change of") } else { ("Remove", "Remove the key") };
        let button = ui.add_enabled(self.editable, egui::Button::new(label).small());
        widgets::name(&button, &format!("{what} {name}"));
        if button.clicked() {
            act = Some(if changed { Act::Undo(name.to_owned()) } else { Act::Remove(name.to_owned()) });
        }
        let copy = ui.small_button("Copy");
        widgets::name(&copy, &format!("Copy the value of {name}"));
        if copy.clicked() {
            ui.ctx().copy_text(text::copied(key.value));
            act = Some(Act::Copied(text::copy_note(name, key.value)));
        }
        // A key that goes away shows no value.
        if self.secret && key.state != KeyState::Removed {
            let (label, what) = if shown { ("Hide", "Hide the value of") } else { ("Show", "Show the value of") };
            let button = ui.small_button(label);
            widgets::name(&button, &format!("{what} {name}"));
            if button.clicked() {
                editors.show(name, !shown);
            }
        }
        act
    }

    /// The value of a key that stays: dots, the editor, a long text or a binary value.
    fn value(&self, ui: &mut Ui, editors: &mut Editors, key: &DraftKey<'_>, shown: bool) -> Option<Act> {
        let palette = self.palette;
        if !shown {
            ui.label(RichText::new(HIDDEN).monospace().color(palette.text_faint));
            return None;
        }
        let Some(value) = key.value.text() else {
            ui.label(RichText::new(text::hex(key.value.bytes())).monospace().color(palette.text_muted));
            return None;
        };
        if key.value.len() > EDIT_BYTES || text::lines(value) > EDIT_LINES {
            preview(ui, palette, value);
            return None;
        }
        let rows = text::lines(value).clamp(1, EDITOR_ROWS);
        let field = |buffer| {
            TextEdit::multiline(buffer).id_salt("value").code_editor().desired_width(f32::INFINITY).desired_rows(rows)
        };
        let response = if self.editable {
            ui.add(field(editors.buffer(key.name, value)))
        } else {
            let mut read_only = value;
            ui.add(field(&mut read_only))
        };
        widgets::name(&response, &format!("Value of {}", key.name));
        response.changed().then(|| Act::Edited(key.name.to_owned()))
    }
}

/// The first lines of a text that is too long for the editor, with a note.
fn preview(ui: &mut Ui, palette: &Palette, value: &str) {
    let first: String = value.split_inclusive('\n').take(PREVIEW_LINES).collect();
    ui.label(RichText::new(short(&first, 8000)).monospace().color(palette.text));
    let size = text::size(value.len());
    let note = format!("{} lines, {size}: too long for the editor. Copy gives the whole text.", text::lines(value));
    ui.label(RichText::new(note).color(palette.text_muted));
}

/// The mark of the state of a key. A key of the read with no change has none.
fn state(state: KeyState) -> Option<(Tone, &'static str)> {
    match state {
        KeyState::Same => None,
        KeyState::Changed => Some((Tone::Info, "changed")),
        KeyState::Added => Some((Tone::Ok, "new")),
        KeyState::Removed => Some((Tone::Warn, "removed")),
    }
}
