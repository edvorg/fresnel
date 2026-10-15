//! The actions on the object of the panel: the actions that this version runs, and the
//! buttons of the header. The command table has each action; a later milestone gives the
//! other ones their requests, and their entries are off until then.
//!
//! The header has a button for each operation of the kind and for Delete, each one with its
//! gate, and the menu "More" with each entry of the command table for the kind. The keys of a
//! Secret or a ConfigMap have their own controls in the key view.

use egui::containers::menu::{MenuButton, MenuConfig};
use egui::{Button, PopupCloseBehavior, RichText, Ui};
use fresnel_core::commands::{self, CommandId, Confirm, Keys};
use fresnel_core::kinds::{Action, Kind};

use crate::shell::keys;
use crate::theme::SMALL;
use crate::views::act::Gate;

/// True for an action that this version runs.
pub(crate) fn can_run(action: Action) -> bool {
    matches!(
        action,
        Action::ViewManifest
            | Action::CopyName
            | Action::CopyCommand
            | Action::EditKeys
            | Action::AddKey
            | Action::RemoveKey
            | Action::Scale
            | Action::Restart
            | Action::RolloutStatus
            | Action::RevisionHistory
            | Action::Rollback
            | Action::Delete
            | Action::ForceDelete
            | Action::Evict
            | Action::RunAgain
            | Action::Suspend
            | Action::Resume
            | Action::Trigger
            | Action::Cordon
            | Action::Uncordon
            | Action::Drain
    )
}

/// Why an action does nothing in this version.
pub(crate) fn later(action: Action) -> String {
    format!("{} comes with a later milestone of the app.", action.label())
}

/// True for an action that also works on an object that its store does not have any more.
pub(crate) fn works_on_gone(action: Action) -> bool {
    matches!(action, Action::ViewManifest | Action::CopyName | Action::CopyCommand)
}

/// True for an action with a button of its own in the header: an operation of a job. A copy
/// and the manifest are in the menu and in the tabs, the keys in the key view.
fn on_header(action: Action) -> bool {
    let elsewhere = matches!(
        action,
        Action::ViewManifest
            | Action::CopyName
            | Action::CopyCommand
            | Action::ForceDelete
            | Action::EditKeys
            | Action::AddKey
            | Action::RemoveKey
    );
    can_run(action) && !elsewhere
}

/// The words of an entry: an action that changes the cluster ends with `…`, because it asks
/// before it acts.
pub(crate) fn label(action: Action) -> String {
    match commands::entry(CommandId::Object(action)).map(|entry| entry.confirm) {
        Some(Confirm::Always) => format!("{}…", action.label()),
        Some(Confirm::Never) | None => action.label().to_owned(),
    }
}

/// The actions of a kind, in the sequence of the command table.
pub(crate) fn of_kind(kind: Kind) -> impl Iterator<Item = Action> {
    commands::TABLE.iter().filter_map(move |entry| match entry.id {
        CommandId::Object(action) if entry.kinds().any(|of_entry| of_entry == kind) => Some(action),
        _ => None,
    })
}

/// Draws the buttons of the header and the menu. `gate` gives the gate of each action.
/// Gives the action of a click.
pub(crate) fn bar(ui: &mut Ui, kind: Kind, keys: &Keys, mut gate: impl FnMut(Action) -> Gate) -> Option<Action> {
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for action in of_kind(kind).filter(|action| on_header(*action)) {
            let gate = gate(action);
            let button = Button::new(RichText::new(label(action)).size(SMALL));
            let mut response = ui.add_enabled(!gate.is_closed(), button);
            if let Some(hint) = gate.hint() {
                response = response.on_hover_text(&hint).on_disabled_hover_text(hint);
            }
            if response.clicked() {
                chosen = Some(action);
            }
        }
        let config = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
        let (_, inner) = MenuButton::new(RichText::new("More").size(SMALL))
            .config(config)
            .ui(ui, |ui| entries(ui, kind, keys, &mut gate));
        chosen = chosen.or(inner.and_then(|inner| inner.inner));
    });
    chosen
}

/// The entries of a menu of an object of `kind`: each action of the command table, with
/// its key and its gate. Gives the action of a click.
pub(crate) fn entries(ui: &mut Ui, kind: Kind, keys: &Keys, mut gate: impl FnMut(Action) -> Gate) -> Option<Action> {
    let mut chosen = None;
    for action in of_kind(kind) {
        let gate = gate(action);
        let mut button = Button::new(label(action));
        if let Some(key) = keys::text(ui.ctx(), keys, CommandId::Object(action)) {
            button = button.shortcut_text(key);
        }
        let mut response = ui.add_enabled(!gate.is_closed(), button);
        if let Some(hint) = gate.hint() {
            response = response.on_hover_text(&hint).on_disabled_hover_text(hint);
        }
        if response.clicked() {
            chosen = Some(action);
            ui.close();
        }
    }
    chosen
}
