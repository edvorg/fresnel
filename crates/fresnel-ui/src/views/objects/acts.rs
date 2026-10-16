//! The actions of a page (plan sections 10 and 11.5): the action that a key, a menu, the
//! quick search or the header of the panel asked for in this frame, the dialog that asks
//! before a change, and the bulk actions on the selected rows.
//!
//! An action that only reads runs at once as a job. An action that changes a cluster opens
//! the confirm dialog. A closed gate gives a note with its reason, and nothing goes.

use fresnel_core::app::{Command, CoreView};
use fresnel_core::kinds::{Action, Describe, Kind, Object};
use fresnel_core::ops::{Order, Target};

use super::follow::Follower;
use super::uid;
use crate::shell::Scene;
use crate::views::act::{
    Ask, Close, Commands, ConfirmDialog, Dialog, DrainDialog, Found, Gate, Permits, Subject, asks_first_on, facts,
    operation, over_room,
};
use crate::views::detail::DetailPanel;
use crate::views::detail::actions::{later, of_kind};
use crate::views::logs::LogAsk;
use crate::widgets::tone::Tone;

/// A note of the page: its tone and its text.
pub(super) type Note = (Tone, String);

/// The bulk actions of plan section 10, for the kinds that have them.
const BULK: [Action; 3] = [Action::Scale, Action::Restart, Action::Delete];

/// The action of this frame, the open dialog, and what the menus asked of the permissions.
#[derive(Default)]
pub(super) struct Acts {
    asked: Option<Ask>,
    /// The choice of a log view in the panel, with an ask of `Action::Logs`.
    logs: Option<LogAsk>,
    dialog: Option<Dialog>,
    permits: Permits,
}

impl Acts {
    /// The user asked for an action on the object of the panel. It starts in the frame.
    pub(super) fn ask(&mut self, ask: Ask) {
        (self.asked, self.logs) = (Some(ask), None);
    }

    /// The user asked for the logs of a container of the object of the panel.
    pub(super) fn ask_logs(&mut self, logs: LogAsk) {
        (self.asked, self.logs) = (Some(Ask { action: Action::Logs, revision: None }), Some(logs));
    }

    pub(super) fn has_dialog(&self) -> bool {
        self.dialog.is_some()
    }

    /// Drops the action and the dialog of a page that is not painted.
    pub(super) fn rest(&mut self) {
        (self.asked, self.logs, self.dialog) = (None, None, None);
    }

    /// The gate of an action on the object of a row, for the menu of the row.
    pub(super) fn gate(&mut self, view: &CoreView, subject: &Subject, action: Action, asks: &mut Vec<Command>) -> Gate {
        let decide = || self.permits.decide(view, &subject.target, action, asks);
        Gate::in_cluster(view, &subject.target.cluster, (action, &subject.facts), decide)
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
        if action == Action::Drain {
            self.dialog = Some(Dialog::Drain(DrainDialog::new(subject)));
            return None;
        }
        if action == Action::Logs {
            scene.logs = Some(self.logs.take().unwrap_or_default().request(&subject.target));
            return None;
        }
        // The editor asks before a change: its save shows the diff.
        if action == Action::EditManifest {
            scene.edit = Some(subject.target);
            return None;
        }
        if asks_first_on(action, subject.target.kind) {
            self.dialog = Some(Dialog::Confirm(ConfirmDialog::new(action, vec![subject], ask.revision)));
            return None;
        }
        let Some(operation) = operation(action) else {
            return Some((Tone::Warn, later(action)));
        };
        if let Some(reason) = over_room(1, scene.view.jobs.room()) {
            return Some((Tone::Warn, format!("{} did not run: {reason}", action.label())));
        }
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
    /// the gate of one of them is closed, with its object in the reason, and for more objects
    /// than the jobs that the app can start now. `selected` gives them: up to that room, the
    /// work of a frame for each one stays small.
    pub(super) fn bulk_gates(
        &mut self,
        kind: Kind,
        count: usize,
        selected: impl FnOnce() -> Vec<Subject>,
        view: &CoreView,
        asks: &mut Vec<Command>,
    ) -> Vec<(Action, Gate)> {
        let actions: Vec<Action> = BULK.into_iter().filter(|action| of_kind(kind).any(|of| of == *action)).collect();
        if let Some(reason) = over_room(count, view.jobs.room()) {
            return actions.into_iter().map(|action| (action, Gate::Closed(reason.clone()))).collect();
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

    /// Draws the open dialog, which finds its objects in the stores of `follower`. Gives the
    /// note of its end.
    pub(super) fn show_dialog<K: Describe>(
        &mut self,
        ctx: &egui::Context,
        scene: &mut Scene<'_>,
        follower: &Follower<K>,
    ) -> Option<Note> {
        let dialog = self.dialog.as_mut()?;
        let note = match dialog.show(ctx, scene, &|target| found(follower, target)) {
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

/// The object of a target in the snapshot of its cluster: a new object of the same name is an
/// other object, and only a complete list tells that an object is gone (design rule 5).
fn found<K: Describe>(follower: &Follower<K>, target: &Target) -> Found {
    let Some(snapshot) = follower.snapshot(&target.cluster) else {
        return Found::NotKnown;
    };
    match snapshot.items.get(&target.key) {
        Some(object) if uid(object.meta()) == &*target.uid => Found::Here(facts::of(&**object)),
        Some(_) => Found::Gone("A new object with the same name took its place.".to_owned()),
        None if snapshot.listed => Found::Gone("It is not in its store now.".to_owned()),
        None => Found::NotKnown,
    }
}
