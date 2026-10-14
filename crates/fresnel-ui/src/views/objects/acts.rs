//! The actions of a page (plan sections 10 and 11.5): the action that a key, a menu, the
//! quick search or the header of the panel asked for in this frame, the dialog that asks
//! before a change, and the bulk actions on the selected rows.
//!
//! An action that only reads runs at once as a job. An action that changes a cluster opens
//! the confirm dialog. A closed gate gives a note with its reason, and nothing goes.

use fresnel_core::app::{Command, CoreView};
use fresnel_core::kinds::{Action, Describe, Kind};
use fresnel_core::ops::Order;

use crate::shell::Scene;
use crate::views::act::{Ask, Close, Commands, ConfirmDialog, Dialog, Gate, Permits, Subject, asks_first, operation};
use crate::views::detail::DetailPanel;
use crate::views::detail::actions::{later, of_kind};
use crate::widgets::tone::Tone;

/// A note of the page: its tone and its text.
pub(super) type Note = (Tone, String);

/// The bulk actions of plan section 10, for the kinds that have them.
const BULK: [Action; 3] = [Action::Scale, Action::Restart, Action::Delete];
/// The most selected objects whose gates the bar decides in a frame: each decision reads the
/// permissions of one object. With more, the buttons stay on, and the dialog decides each one.
const BULK_CHECKS: usize = 1000;

/// The action of this frame, the open dialog, and what the menus asked of the permissions.
#[derive(Default)]
pub(super) struct Acts {
    asked: Option<Ask>,
    dialog: Option<Dialog>,
    permits: Permits,
}

impl Acts {
    /// The user asked for an action on the object of the panel. It starts in the frame.
    pub(super) fn ask(&mut self, ask: Ask) {
        self.asked = Some(ask);
    }

    pub(super) fn has_dialog(&self) -> bool {
        self.dialog.is_some()
    }

    /// Drops the action and the dialog of a page that is not painted.
    pub(super) fn rest(&mut self) {
        (self.asked, self.dialog) = (None, None);
    }

    /// The gate of an action on the object of a row, for the menu of the row.
    pub(super) fn gate(&mut self, view: &CoreView, subject: &Subject, action: Action, asks: &mut Vec<Command>) -> Gate {
        Gate::of(action, &subject.facts, || self.permits.decide(view, &subject.target, action, asks))
    }

    /// Starts the action that the user asked for on the object of the panel: a dialog, a
    /// job, or a note that tells why not.
    pub(super) fn begin<K: Describe>(
        &mut self,
        panel: Option<&mut DetailPanel<K>>,
        scene: &mut Scene<'_>,
    ) -> Option<Note> {
        let ask = self.asked.take()?;
        let action = ask.action;
        let Some(panel) = panel else {
            return Some((
                Tone::Warn,
                format!("No object is open in the panel. {} acts on that object.", action.label()),
            ));
        };
        let subject = panel.subject();
        if action == Action::CopyCommand {
            self.dialog = Some(Dialog::Commands(Commands::of(&subject)));
            return None;
        }
        if let Gate::Closed(reason) = panel.gate(action, scene) {
            return Some((Tone::Warn, format!("{} did not run: {reason}", action.label())));
        }
        if asks_first(action) {
            self.dialog = Some(Dialog::Confirm(ConfirmDialog::new(action, vec![subject], ask.revision)));
            return None;
        }
        let Some(operation) = operation(action) else {
            return Some((Tone::Warn, later(action)));
        };
        let text = format!("Started: {} of {}. The Operations tab shows the job.", operation.label(), subject.text());
        match Order::new(subject.target, operation) {
            Ok(order) => {
                scene.commands.push(Command::Run(vec![order]));
                Some((Tone::Info, text))
            }
            Err(error) => Some((Tone::Warn, error.to_string())),
        }
    }

    /// The gate of each bulk action of `kind` on the `count` selected objects: closed when
    /// the gate of one of them is closed, with its object in the reason. `selected` gives
    /// them, and the bar asks for them only up to its limit.
    pub(super) fn bulk_gates(
        &mut self,
        kind: Kind,
        count: usize,
        selected: impl FnOnce() -> Vec<Subject>,
        view: &CoreView,
        asks: &mut Vec<Command>,
    ) -> Vec<(Action, Gate)> {
        let actions: Vec<Action> = BULK.into_iter().filter(|action| of_kind(kind).any(|of| of == *action)).collect();
        if count > BULK_CHECKS {
            let text = format!("the dialog decides each of the {count} objects");
            return actions.into_iter().map(|action| (action, Gate::Unknown(text.clone()))).collect();
        }
        let selected = selected();
        let gate = |action: Action, acts: &mut Self, asks: &mut Vec<Command>| {
            let mut unknown = None;
            for subject in &selected {
                match acts.gate(view, subject, action, asks) {
                    Gate::Open => {}
                    Gate::Unknown(reason) => unknown = unknown.or(Some(reason)),
                    Gate::Closed(reason) => return Gate::Closed(format!("{}: {reason}", subject.text())),
                }
            }
            unknown.map_or(Gate::Open, Gate::Unknown)
        };
        actions.into_iter().map(|action| (action, gate(action, self, asks))).collect()
    }

    /// Opens the confirm dialog of a bulk action on the selected objects.
    pub(super) fn bulk(&mut self, action: Action, mut subjects: Vec<Subject>) {
        let key = |subject: &Subject| (subject.target.cluster.clone(), subject.target.key.clone());
        subjects.sort_by_key(key);
        self.dialog = Some(Dialog::Confirm(ConfirmDialog::new(action, subjects, None)));
    }

    /// Draws the open dialog. Gives the note of its end.
    pub(super) fn show_dialog(&mut self, ctx: &egui::Context, scene: &mut Scene<'_>) -> Option<Note> {
        let dialog = self.dialog.as_mut()?;
        let note = match dialog.show(ctx, scene) {
            Close::Open => return None,
            Close::Cancelled => None,
            Close::Run { orders, note } if orders.is_empty() => Some((Tone::Warn, note)),
            Close::Run { orders, note } => {
                scene.commands.push(Command::Run(orders));
                Some((Tone::Info, note))
            }
            Close::Copied(command) => Some((Tone::Info, format!("Copied: {command}"))),
        };
        self.dialog = None;
        ctx.request_repaint();
        note
    }
}
