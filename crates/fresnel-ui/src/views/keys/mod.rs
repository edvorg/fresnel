//! The keys of a Secret or a ConfigMap in the detail panel (plan section 12.2): an editor
//! for each key, show and hide for each value of a Secret, copy, add and remove a key, the
//! diff of the changed keys before a save, the state of the save, the conflicts with the
//! server, the marks of an immutable object and of a binary value, and the views of the
//! certificates of a TLS Secret and the registries of an image pull Secret.
//!
//! No store holds a value of a Secret. The core reads the object of the panel while the
//! panel is open (`Command::ShowKeys`), and the panel edits a `Draft` on that read. A new
//! read takes the place of a draft with no change. A draft with changes moves onto it, and
//! each key that the user and the server both changed is a conflict. A save sends only the
//! changed keys, with the version of the read, so each other key stays equal byte for byte.
//!
//! - `list`: the keys of the rows. `rows`: one row for each key. `add`: the row of a new key.
//! - `text`: the texts of a value.
//! - `confirm`: the dialog of a save, with the `diff` of each key. `state`: read and save.
//! - `conflicts`: a choice for each conflict. `special`: certificates and registries.
//! - `new_secret`: the dialog of a new Secret.

mod add;
mod confirm;
mod conflicts;
mod diff;
mod list;
pub(crate) mod new_secret;
mod rows;
mod special;
mod state;
mod text;

use std::sync::Arc;

use egui::{Align, RichText, Ui};
use fresnel_core::app::{ClusterData, Command, KeysView};
use fresnel_core::keys::{Conflict, Draft, KeyKind, KeyObject, Special, Target, WriteState};
use fresnel_core::kinds::Action;
use fresnel_core::store::Written;

use self::add::AddKey;
use self::confirm::{Answer, Confirm};
use self::conflicts::Choice;
use self::list::KeyList;
use self::rows::{Act, Editors, Rows};
use crate::format;
use crate::shell::Scene;
use crate::theme::Palette;
use crate::views::act::Gate;
use crate::widgets::tone::Tone;

/// The keys of the object of a detail panel, with the edit of the user.
pub(crate) struct KeyPanel {
    target: Target,
    /// The edit on the last read. `None` before the first read.
    draft: Option<Draft>,
    /// A number that grows with each change of the draft, for the results that the panel
    /// keeps: the names of the rows and the special view.
    serial: u64,
    editors: Editors,
    add: AddKey,
    confirm: Option<Confirm>,
    conflicts: Vec<Conflict>,
    list: KeyList,
    special: Option<(u64, Special)>,
    /// What the last copy or refusal did, with no value.
    note: Option<(Tone, String)>,
    /// An action of the command table for the next frame: the section scrolls into view.
    focus: Option<Action>,
}

impl KeyPanel {
    pub(crate) fn new(target: Target) -> Self {
        Self {
            target,
            draft: None,
            serial: 0,
            editors: Editors::default(),
            add: AddKey::default(),
            confirm: None,
            conflicts: Vec::new(),
            list: KeyList::default(),
            special: None,
            note: None,
            focus: None,
        }
    }

    /// The object whose values the core reads for the panel.
    pub(crate) fn target(&self) -> &Target {
        &self.target
    }

    /// True while the dialog of a save is open: the keys of the shell wait, as for the
    /// dialog of an action, so no command opens a view over it or closes the panel below it.
    pub(crate) fn has_dialog(&self) -> bool {
        self.confirm.is_some()
    }

    /// The result of the last save that the server stored, for the mark "change pending"
    /// of the panel until the watch brings that version (plan section 9.10).
    pub(crate) fn written(&self, data: Option<&ClusterData>) -> Option<Written> {
        let view = data?.keys.as_deref().filter(|view| view.target == self.target)?;
        match &view.save.as_ref()?.state {
            WriteState::Done(object) => object.written(),
            _ => None,
        }
    }

    /// Runs an action of the command table on the keys: the next frame shows the section,
    /// and "Add key" puts the focus into the name of a new key. Gives a note.
    pub(crate) fn act(&mut self, action: Action) -> Option<String> {
        self.focus = Some(action);
        match action {
            Action::AddKey => self.add.focus(),
            Action::RemoveKey => return Some("Each key has its Remove button in the section DATA.".to_owned()),
            _ => {}
        }
        None
    }

    /// Draws the section of the keys. `data` is the data of the cluster of the object, or
    /// `None` while the cluster is not connected. `gate` is the gate of a change of the keys
    /// in the panel: it closes for a change that the rules forbid and for an object that
    /// left its store, as for each other action of the panel (plan section 11.2).
    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        palette: &Palette,
        scene: &mut Scene<'_>,
        data: Option<&ClusterData>,
        gate: &Gate,
    ) {
        let view = data.and_then(|data| data.keys.as_deref()).filter(|view| view.target == self.target);
        if let Some(object) = view.and_then(|view| view.object.as_ref()) {
            self.take(object);
        }
        let title = state::heading(ui, palette, self.draft.as_ref());
        if matches!(self.focus.take(), Some(Action::EditKeys | Action::RemoveKey)) {
            title.scroll_to_me(Some(Align::TOP));
        }
        let version = self.draft.as_ref().map(|draft| &*draft.base().version);
        state::read(ui, palette, &self.target.cluster, data.is_some(), view, version);
        let Some(draft) = &self.draft else {
            return;
        };
        let kind = self.target.kind.kind().info().name;
        let lock = lock(kind, draft.base().immutable, gate);
        if let Some(text) = &lock {
            ui.label(RichText::new(text).color(palette.warn));
        }
        // An edit while a save is on its way would meet the answer of that save as a change
        // of the server: a conflict of the user with the own save.
        let editable = lock.is_none() && !saving(view);
        let secret = self.target.kind == KeyKind::Secret;
        // The bar is above the keys, so a long list does not hide it or the state of a save.
        self.save_bar(ui, palette, lock.is_none(), view);
        if let Some(job) = view.and_then(|view| view.save.as_ref()) {
            state::save(ui, palette, kind, job);
        }
        if let Some((tone, text)) = &self.note {
            ui.label(RichText::new(text).color(tone.color(palette)));
        }
        if let Some(choice) = conflicts::show(ui, palette, &self.conflicts, &mut self.editors, secret) {
            self.choose(choice);
        }
        ui.add_space(4.0);
        self.rows(ui, palette, editable, secret);
        if let Some((name, value)) = self.add.show(ui, palette, editable) {
            self.add_key(&name, value);
        }
        let refusal = lock.or_else(|| self.wait(view).map(str::to_owned));
        self.dialog(ui, palette, scene, secret, refusal.as_deref());
        self.special(ui, palette, scene);
    }

    /// Takes a new read of the core. A draft with no change takes its place; a draft with
    /// changes moves onto it and gives the conflicts.
    fn take(&mut self, object: &Arc<KeyObject>) {
        let Some(draft) = &mut self.draft else {
            self.draft = Some(Draft::new(Arc::clone(object)));
            self.changed();
            return;
        };
        if Arc::ptr_eq(draft.base(), object) {
            return;
        }
        if draft.is_changed() {
            match draft.rebase(Arc::clone(object)) {
                Ok(conflicts) => self.conflicts = conflicts,
                Err(refusal) => self.note = Some((Tone::Warn, format!("The new read does not fit: {refusal}"))),
            }
        } else {
            *draft = Draft::new(Arc::clone(object));
        }
        let changes = draft.changes();
        self.editors.forget(|key| changes.contains_key(key));
        self.changed();
    }

    /// The draft changed: the results that the panel keeps are old, and a conflict whose
    /// key has no change any more is gone.
    fn changed(&mut self) {
        self.serial += 1;
        if let Some(draft) = &self.draft {
            self.conflicts.retain(|conflict| draft.changes().contains_key(&conflict.key));
        }
    }

    fn choose(&mut self, choice: Choice) {
        match choice {
            Choice::Mine(key) => self.conflicts.retain(|conflict| conflict.key != key),
            Choice::Server(key) => self.undo(&key),
        }
    }

    fn undo(&mut self, key: &str) {
        if let Some(draft) = &mut self.draft {
            draft.undo(key);
            self.editors.forget(|held| held != key);
            self.changed();
        }
    }

    /// The filter, a row for each key that passes it, and the count of the rows that do not show.
    fn rows(&mut self, ui: &mut Ui, palette: &Palette, editable: bool, secret: bool) {
        let Some(draft) = &self.draft else {
            return;
        };
        let rows = Rows { palette, editable, secret };
        let mut acts = Vec::new();
        for name in self.list.names(ui, palette, draft, self.serial) {
            if let Some(key) = draft.key(&name) {
                let place = draft.base().entries.get(&name).map(|entry| entry.place);
                acts.extend(rows.show(ui, &mut self.editors, &key, place));
            }
        }
        self.list.more(ui, palette);
        for act in acts {
            self.apply(act);
        }
    }

    fn apply(&mut self, act: Act) {
        let Some(draft) = &mut self.draft else {
            return;
        };
        let result = match &act {
            Act::Edited(key) => {
                let bytes = self.editors.text(key).unwrap_or_default().as_bytes().to_vec();
                draft.set(key, bytes)
            }
            Act::Remove(key) => draft.remove(key),
            Act::Undo(key) => {
                self.undo(key);
                return;
            }
            Act::Copied(note) => {
                self.note = Some((Tone::Info, note.clone()));
                return;
            }
        };
        self.note = result.err().map(|refusal| (Tone::Error, refusal.to_string()));
        if let Act::Remove(key) = &act {
            self.editors.forget(|held| held != key);
        }
        self.changed();
    }

    fn add_key(&mut self, name: &str, value: Vec<u8>) {
        let Some(draft) = &mut self.draft else {
            return;
        };
        match draft.add(name, value) {
            Ok(()) => {
                self.add.clear();
                self.editors.show(name, true);
                self.changed();
            }
            Err(refusal) => self.add.refused(refusal.to_string()),
        }
    }

    /// Why a save of an object that the user can change cannot start now. With no view of
    /// the core, its cluster is not connected and a save would not go: the core has no holder.
    fn wait(&self, view: Option<&KeysView>) -> Option<&'static str> {
        if view.is_none() {
            Some("The cluster of the object is not connected.")
        } else if saving(view) {
            Some("A save is on its way.")
        } else if !self.conflicts.is_empty() {
            Some("Choose a value for each conflict first.")
        } else {
            None
        }
    }

    /// The buttons Save and Discard, and why a save cannot start now. `unlocked` is false
    /// for an object that the user cannot change; the reason shows above the bar. During a
    /// save, a discard would also meet its answer, so it waits too.
    fn save_bar(&mut self, ui: &mut Ui, palette: &Palette, unlocked: bool, view: Option<&KeysView>) {
        let wait = self.wait(view);
        let Some(draft) = &self.draft else {
            return;
        };
        let changes = draft.changes().len();
        let base = Arc::clone(draft.base());
        let mut discard = false;
        ui.horizontal(|ui| {
            let label = if changes == 0 {
                "Save".to_owned()
            } else {
                format!("Save {}…", format::count(changes as u64, "key"))
            };
            let save = ui.add_enabled(unlocked && changes > 0 && wait.is_none(), egui::Button::new(label));
            if save.clicked() {
                match draft.save() {
                    Ok(save) => self.confirm = Some(Confirm::new(save, draft.base())),
                    Err(refusal) => self.note = Some((Tone::Error, refusal.to_string())),
                }
            }
            discard = ui.add_enabled(changes > 0 && !saving(view), egui::Button::new("Discard")).clicked();
            if let Some(reason) = wait.filter(|_| changes > 0) {
                ui.label(RichText::new(reason).color(palette.text_muted));
            }
        });
        if discard {
            self.draft = Some(Draft::new(base));
            self.conflicts.clear();
            self.editors.forget(|_| false);
            self.changed();
        }
    }

    /// The dialog of a save while it is open. A save of the dialog goes to the core, unless
    /// `refusal` tells why it cannot go now: while the dialog is open, the object can leave
    /// its store, the rules can forbid the change and the cluster can go.
    fn dialog(&mut self, ui: &mut Ui, palette: &Palette, scene: &mut Scene<'_>, secret: bool, refusal: Option<&str>) {
        let Some(confirm) = &mut self.confirm else {
            return;
        };
        let color = scene.colors.color(&self.target.cluster, palette);
        match confirm.show(ui.ctx(), (color, secret), refusal, scene.view) {
            Some(Answer::Save) => {
                scene.commands.push(Command::SaveKeys(confirm.save().clone()));
                self.confirm = None;
            }
            Some(Answer::Cancel) => self.confirm = None,
            None => {}
        }
    }

    /// The certificates or the registries of the values of the draft.
    fn special(&mut self, ui: &mut Ui, palette: &Palette, scene: &Scene<'_>) {
        let Some(draft) = &self.draft else {
            return;
        };
        if self.special.as_ref().is_none_or(|(serial, _)| *serial != self.serial) {
            self.special = Some((self.serial, draft.special()));
        }
        if let Some((_, special)) = &self.special {
            special::show(ui, palette, scene.clock, special);
        }
    }
}

/// Why the user cannot change the keys: an immutable object, or the closed gate of the
/// change. `None` when the user can change them.
fn lock(kind: &str, immutable: bool, gate: &Gate) -> Option<String> {
    if immutable {
        return Some(format!(
            "This {kind} is immutable: the server refuses each change of a value. Only a delete and a new create change it."
        ));
    }
    match gate {
        Gate::Closed(reason) => Some(format!("You cannot change this {kind}. {reason}")),
        Gate::Open | Gate::Unknown(_) => None,
    }
}

/// True while the last save of the view is on its way.
fn saving(view: Option<&KeysView>) -> bool {
    view.and_then(|view| view.save.as_ref()).is_some_and(|job| job.state == WriteState::Pending)
}
