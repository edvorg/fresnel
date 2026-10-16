//! The bar of an editor: the object and the version of its text, the buttons of the undo,
//! the redo, the start text and the search, the state of the check, and the notes of the
//! text; below it the bar of the search while it is open.

use egui::{Button, Key, Layout, Modifiers, RichText, TextEdit, Ui};
use fresnel_core::manifest::{EditText, LINE_LIMIT};

use super::find::Find;
use crate::format::thousands;
use crate::theme::{self, SMALL};
use crate::widgets;
use crate::widgets::tone::Tone;

/// What the user asked for in the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Asked {
    Undo,
    Redo,
    /// Back to the text of the read, as one step of the undo.
    Revert,
    /// The next hit of the search, or the one before.
    Go(bool),
    /// Back to the text from the bar of the search.
    Text,
    /// The dry run of a save, before its dialog.
    Save,
}

/// What the bar shows.
pub(super) struct Parts<'a> {
    /// "Deployment shop/web in ams-prod".
    pub(super) object: &'a str,
    pub(super) read: &'a EditText,
    pub(super) lines: usize,
    pub(super) editable: bool,
    pub(super) changed: bool,
    pub(super) undo: bool,
    pub(super) redo: bool,
    /// The state of the check, with true for an error.
    pub(super) check: Option<(String, bool)>,
    /// The notes of the text and of the tabs, each with its tone.
    pub(super) notes: Vec<(Tone, String)>,
    /// Whether the text can go to a save now, or why not.
    pub(super) save: Result<(), String>,
}

pub(super) fn show(ui: &mut Ui, parts: &Parts<'_>, find: &mut Find) -> Vec<Asked> {
    let palette = theme::palette(ui);
    let mut asked = Vec::new();
    ui.horizontal(|ui| {
        let changed = if parts.changed { " · changed" } else { "" };
        let version = parts.read.version.as_deref().unwrap_or("none");
        let lines = crate::format::count(parts.lines as u64, "line");
        ui.label(RichText::new(parts.object).strong());
        ui.label(RichText::new(format!("version {version} · {lines}{changed}")).size(SMALL).color(palette.text_muted));
        let mut button = |ui: &mut Ui, (text, name, enabled): (&str, &str, bool), ask: Asked| {
            let response = ui.add_enabled(enabled, Button::new(RichText::new(text).size(SMALL)));
            widgets::name(&response, name);
            if response.clicked() {
                asked.push(ask);
            }
        };
        if parts.editable {
            button(ui, ("Undo", "Undo", parts.undo), Asked::Undo);
            button(ui, ("Redo", "Redo", parts.redo), Asked::Redo);
            button(ui, ("Revert", "Revert to the text of the read", parts.changed), Asked::Revert);
        }
        let search = ui.add(Button::new(RichText::new("Find").size(SMALL)).selected(find.open));
        widgets::name(&search, "Find in the text");
        if search.clicked() {
            if find.open { find.open = false } else { find.start() }
        }
        if parts.editable {
            let save = ui.add_enabled(parts.save.is_ok(), Button::new(RichText::new("Save…").size(SMALL)));
            widgets::name(&save, "Save the text");
            if let Err(reason) = &parts.save {
                save.clone().on_disabled_hover_text(reason);
            }
            if save.clicked() {
                asked.push(Asked::Save);
            }
        }
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some((state, error)) = &parts.check {
                let color = if *error { palette.error } else { palette.text_muted };
                ui.label(RichText::new(state).size(SMALL).color(color));
            }
        });
    });
    for (tone, note) in &parts.notes {
        ui.label(RichText::new(note).size(SMALL).color(tone.color(palette)));
    }
    if find.open {
        find_bar(ui, find, &mut asked);
    }
    asked
}

/// The field of the search with the count of its hits and its buttons. Enter goes to the
/// next hit and Shift+Enter to the one before; Escape closes the bar.
pub(super) fn find_bar(ui: &mut Ui, find: &mut Find, asked: &mut Vec<Asked>) {
    let palette = theme::palette(ui);
    ui.horizontal(|ui| {
        let field = TextEdit::singleline(&mut find.text).hint_text("Find").desired_width(220.0);
        let response = ui.add(field);
        widgets::name(&response, "The text to find");
        find.field = Some(response.id);
        if std::mem::take(&mut find.focus) {
            response.request_focus();
        }
        if response.lost_focus() {
            let (enter, shift, escape) = ui
                .input(|input| (input.key_pressed(Key::Enter), input.modifiers.shift, input.key_pressed(Key::Escape)));
            if enter {
                asked.push(Asked::Go(!shift));
                response.request_focus();
            } else if escape {
                find.open = false;
                asked.push(Asked::Text);
            }
        }
        ui.label(RichText::new(find.count()).size(SMALL).color(palette.text_muted));
        for (text, name, forward) in [("‹", "The hit before", false), ("›", "The next hit", true)] {
            let button = ui.small_button(text);
            widgets::name(&button, name);
            if button.clicked() {
                asked.push(Asked::Go(forward));
            }
        }
        let close = ui.small_button("×");
        widgets::name(&close, "Close the search");
        if close.clicked() {
            find.open = false;
            asked.push(Asked::Text);
        }
    });
}

/// The keys of the search and of the marks while the editor has the focus: Cmd+G and
/// Shift+Cmd+G for the hits, F8 and Shift+F8 for the marks. Gives the hit move and the mark move.
pub(super) fn keys(ui: &Ui) -> (Option<bool>, Option<bool>) {
    ui.input_mut(|input| {
        let hit = [(Modifiers::COMMAND | Modifiers::SHIFT, false), (Modifiers::COMMAND, true)]
            .into_iter()
            .find_map(|(modifiers, forward)| input.consume_key(modifiers, Key::G).then_some(forward));
        let mark = [(Modifiers::SHIFT, false), (Modifiers::NONE, true)]
            .into_iter()
            .find_map(|(modifiers, forward)| input.consume_key(modifiers, Key::F8).then_some(forward));
        (hit, mark)
    })
}

/// The notes of a text: the values of a Secret, and a text past the line limit.
pub(super) fn notes(read: &EditText, editable: bool) -> Vec<(Tone, String)> {
    let mut notes = Vec::new();
    if read.decoded.is_some() {
        let note =
            "The text values of the Secret show decoded. A value that is not text stays base64 with the tag !!binary.";
        notes.push((Tone::Info, note.to_owned()));
    }
    if !editable {
        let (lines, limit) = (thousands(read.lines as u64), thousands(LINE_LIMIT as u64));
        let note = format!("The manifest has {lines} lines, more than the {limit} of the editor, so it is read-only.");
        notes.push((Tone::Warn, note));
    }
    notes
}
