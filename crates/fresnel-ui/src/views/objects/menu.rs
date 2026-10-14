//! The menu of a row (plan section 11.5): the actions of the kind from the command table,
//! each with its key. An action that this version does not run is off, with the reason. An
//! action that changes the cluster ends with `…`: it asks before it acts.

use egui::{Button, Ui};
use fresnel_core::commands::{self, CommandId, Confirm, Keys};
use fresnel_core::kinds::{Action, Kind};

use crate::shell::keys;
use crate::views::detail::actions;

/// Draws the entries of the menu of an object of `kind`. Gives the action of a click.
pub(super) fn show(ui: &mut Ui, kind: Kind, keys: &Keys) -> Option<Action> {
    let mut chosen = None;
    for entry in commands::TABLE {
        let CommandId::Object(action) = entry.id else {
            continue;
        };
        if !entry.kinds().any(|of_entry| of_entry == kind) {
            continue;
        }
        let label = match entry.confirm {
            Confirm::Always => format!("{}…", entry.label),
            Confirm::Never => entry.label.to_owned(),
        };
        let mut button = Button::new(label);
        if let Some(key) = keys::text(ui.ctx(), keys, entry.id) {
            button = button.shortcut_text(key);
        }
        let response = ui.add_enabled(actions::can_run(action), button).on_disabled_hover_text(actions::later(action));
        if response.clicked() {
            chosen = Some(action);
            ui.close();
        }
    }
    chosen
}
