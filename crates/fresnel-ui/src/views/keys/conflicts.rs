//! The keys that the user and the server both changed after the read of the draft
//! (research 08 section 8.3, step 6): the value of the server next to the value of the
//! user, and a choice for each key. A save waits for each choice, so a save never writes
//! over a change of the server that the user did not see.

use egui::{RichText, Ui};
use fresnel_core::keys::{Conflict, Value};

use super::rows::Editors;
use super::text;
use crate::format::short;
use crate::theme::Palette;
use crate::widgets;

/// The most lines of a value that the view shows.
const LINES: usize = 12;

/// The choice of the user for one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    /// The draft keeps the value of the user, and a save sends it.
    Mine(String),
    /// The draft takes the value of the server for the key.
    Server(String),
}

/// Draws each conflict. `secret` hides a value until the user shows its key.
pub(super) fn show(
    ui: &mut Ui,
    palette: &Palette,
    conflicts: &[Conflict],
    editors: &mut Editors,
    secret: bool,
) -> Option<Choice> {
    if conflicts.is_empty() {
        return None;
    }
    ui.label(RichText::new("CONFLICTS").size(11.0).color(palette.warn));
    let text = "The server changed these keys after the read, and you changed them too. Choose a value \
                for each one; the save waits for that.";
    ui.label(RichText::new(text).color(palette.warn));
    let mut choice = None;
    for conflict in conflicts {
        let key = conflict.key.as_str();
        let shown = !secret || editors.is_shown(key);
        ui.push_id(("conflict", key), |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(short(key, 120)).monospace().strong());
                if secret {
                    let label = if shown { "Hide" } else { "Show" };
                    let button = ui.small_button(label);
                    widgets::name(&button, &format!("{label} the values of the conflict of {key}"));
                    if button.clicked() {
                        editors.show(key, !shown);
                    }
                }
            });
            side(ui, palette, "Server", conflict.server.as_ref(), shown);
            side(ui, palette, "Yours", conflict.mine.as_ref(), shown);
            ui.horizontal(|ui| {
                let mine = ui.small_button("Keep mine");
                widgets::name(&mine, &format!("Keep my value of {key}"));
                if mine.clicked() {
                    choice = Some(Choice::Mine(key.to_owned()));
                }
                let server = ui.small_button("Take the server value");
                widgets::name(&server, &format!("Take the server value of {key}"));
                if server.clicked() {
                    choice = Some(Choice::Server(key.to_owned()));
                }
            });
        });
        ui.add_space(8.0);
    }
    choice
}

/// One side of a conflict: no key, a hidden value, the first lines of a text, or the marks
/// of a binary value.
fn side(ui: &mut Ui, palette: &Palette, label: &str, value: Option<&Value>, shown: bool) {
    ui.horizontal_top(|ui| {
        ui.label(RichText::new(format!("{label}:")).color(palette.text_muted));
        let Some(value) = value else {
            ui.label(RichText::new("no key").color(palette.text_faint));
            return;
        };
        match value.text() {
            _ if !shown => {
                ui.label(RichText::new(format!("hidden · {}", text::size(value.len()))).color(palette.text_faint));
            }
            Some(text) => {
                let first: String = text.split_inclusive('\n').take(LINES).collect();
                // The line feed at the end of the last line would show as an empty line.
                let first = first.strip_suffix('\n').unwrap_or(&first);
                ui.label(RichText::new(short(first, 2000)).monospace());
            }
            None => {
                let marks: Vec<String> = text::marks(value).into_iter().map(|(_, mark)| mark).collect();
                ui.label(RichText::new(marks.join(" · ")).color(palette.warn));
            }
        }
    });
}
