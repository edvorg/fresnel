//! The keys of a Secret or a ConfigMap (plan section 12.2) on the fake core, with made-up
//! values only.
//!
//! - `scene`: the objects and the view of the core for the panel of one of them.
//! - `editor`: the read, show and hide, copy, an edit with its diff, and the save.
//! - `changes`: add and remove a key, the states of a save, and a conflict.
//! - `marks`: an immutable object, a forbidden change, a binary value, the special views
//!   and "used by". `new_secret`: the dialog of a new Secret. `snapshots`: PNG files.

mod changes;
mod editor;
mod marks;
mod new_secret;
pub(crate) mod scene;
mod snapshots;

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::Command;
use fresnel_core::keys::Save;

use crate::fake::Window;

/// The text that the last frame put on the clipboard.
pub(crate) fn copied(window: &Window) -> Option<String> {
    window.output().platform_output.commands.iter().find_map(|command| match command {
        egui::OutputCommand::CopyText(text) => Some(text.clone()),
        _ => None,
    })
}

/// The last save that the views sent to the core.
pub(crate) fn save_command(window: &Window) -> Option<Save> {
    window.state().sent.iter().rev().find_map(|command| match command {
        Command::SaveKeys(save) => Some(save.clone()),
        _ => None,
    })
}

/// Clicks the widget with this name. The panel scrolls to it first: a click needs it on the
/// screen, and the dock covers the lower part of a long panel.
pub(crate) fn click(window: &mut Window, label: &str) {
    window.get_by_label(label).scroll_to_me();
    window.run();
    window.get_by_label(label).click();
    window.run();
}

/// Replaces the text of the field with this name, as a user does: a click, Cmd+A and the keys.
pub(crate) fn typed(window: &mut Window, field: &str, text: &str) {
    click(window, field);
    window.key_press_modifiers(Modifiers::COMMAND, Key::A);
    window.run();
    window.get_by_label(field).type_text(text);
    window.run();
}
