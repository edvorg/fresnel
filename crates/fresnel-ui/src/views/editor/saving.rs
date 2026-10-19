//! The save of one editor tab (plan section 12.9): Save asks the core for a dry run, the
//! dialog shows its diff, and Save there sends the replace. The tab follows the state of its
//! last save in the view of the core and takes each end one time: the answer of a save is
//! the new read, a conflict whose changes do not overlap goes into the text as a merge, and
//! a conflict whose changes overlap opens the dialog of the three versions.

use std::sync::Arc;

use egui::Context;
use fresnel_core::app::{Command, CoreView, EditRequest, EditSave, Remote, SaveState, SaveView, Stage};
use fresnel_core::manifest::EditText;

use super::conflict::{self, ConflictDialog};
use super::dialog::{self, SaveDialog};
use super::differ::Differ;
use crate::widgets::tone::Tone;

/// What the tab takes from the end of a save.
#[derive(Clone, Debug)]
pub(super) enum Taken {
    /// The text of the server is the new read and the new text.
    Saved(Arc<EditText>),
    /// A new read, and the text of the merge with it.
    Merged(Arc<EditText>, String),
}

/// The saves of one tab.
#[derive(Default)]
pub(super) struct Saving {
    /// The number of the last save that the tab sent, and its stage.
    number: u64,
    stage: Option<Stage>,
    /// The number of the last save whose end the tab took.
    taken: u64,
    dialog: Option<SaveDialog>,
    conflict: Option<ConflictDialog>,
    /// The words of the last end, with their tone.
    pub(super) note: Option<(Tone, String)>,
}

impl Saving {
    /// The command of a dry run of `text` on the read `base`.
    pub(super) fn check(&mut self, editor: &EditRequest, base: &Arc<EditText>, text: String) -> Command {
        self.number += 1;
        self.stage = Some(Stage::Check);
        self.note = None;
        let save = EditSave {
            editor: editor.clone(),
            number: self.number,
            base: Arc::clone(base),
            text: text.into(),
            stage: Stage::Check,
        };
        Command::SaveEditor(save)
    }

    /// The stage of the last save of the tab while its end did not come: from the click on,
    /// before the core shows it as pending.
    pub(super) fn pending(&self) -> Option<Stage> {
        self.stage.filter(|_| self.number > self.taken)
    }

    /// True while a dialog of the tab is open.
    pub(super) fn has_dialog(&self) -> bool {
        self.dialog.is_some() || self.conflict.is_some()
    }

    /// Takes the end of the last save, one time, and draws the dialogs. Gives the commands
    /// of the user and what the tab takes into its text. `differ` computes the rows of a diff.
    pub(super) fn follow(
        &mut self,
        ctx: &Context,
        (view, saved): (&CoreView, Option<&SaveView>),
        color: egui::Color32,
        differ: &mut Differ,
    ) -> (Option<Command>, Option<Taken>) {
        let mut taken = None;
        if let Some(saved) = saved.filter(|saved| saved.save.number == self.number && self.number > self.taken)
            && saved.state != SaveState::Pending
        {
            self.taken = self.number;
            taken = self.end(&saved.save, &saved.state, differ);
        }
        let mut command = None;
        if let Some(dialog) = &mut self.dialog {
            match dialog.show(ctx, color, view, differ) {
                Some(dialog::Answer::Save) => {
                    let mut save = dialog.save().clone();
                    self.number += 1;
                    (save.number, save.stage) = (self.number, Stage::Send);
                    self.stage = Some(Stage::Send);
                    command = Some(Command::SaveEditor(save));
                    self.dialog = None;
                }
                Some(dialog::Answer::Cancel) => self.dialog = None,
                None => {}
            }
        }
        if let Some(conflict) = &mut self.conflict {
            match conflict.show(ctx) {
                Some(conflict::Answer::Use(text)) => {
                    let read = Arc::clone(&conflict.remote().read);
                    taken = Some(Taken::Merged(read, text));
                    self.note = Some((Tone::Warn, "The text has the merge: check it and save again.".to_owned()));
                    self.conflict = None;
                }
                Some(conflict::Answer::Cancel) => self.conflict = None,
                None => {}
            }
        }
        (command, taken)
    }

    /// The end of a save: a note, a dialog, or what the text takes.
    fn end(&mut self, save: &EditSave, state: &SaveState, differ: &mut Differ) -> Option<Taken> {
        let version = |text: &EditText| text.version.clone().unwrap_or_else(|| "none".to_owned());
        match state {
            SaveState::Pending => None,
            SaveState::Checked(after) => {
                self.dialog = Some(SaveDialog::new(save.clone(), after, differ));
                None
            }
            SaveState::Saved(answer) => {
                self.note = Some((Tone::Ok, format!("Saved: the object has version {} now.", version(answer))));
                Some(Taken::Saved(Arc::clone(answer)))
            }
            SaveState::Conflict(remote) => {
                let words =
                    format!("The object changed on the server since the read: version {} now.", version(&remote.read));
                self.merge(save, remote, ("The object changed on the server", words))
            }
            SaveState::Unknown(error, Some(remote)) => {
                let words = format!(
                    "The result of the save is not known ({error}). The object has version {} now.",
                    version(&remote.read)
                );
                self.merge(save, remote, ("The result of the save is not known", words))
            }
            SaveState::Unknown(error, None) => {
                let words = format!("The result of the save is not known: {error}. The app did not send it again.");
                self.note = Some((Tone::Error, words));
                None
            }
            SaveState::Failed(error) => {
                self.note = Some((Tone::Error, format!("The save was refused: {error}")));
                None
            }
            SaveState::NotSent(error) => {
                self.note = Some((Tone::Error, format!("The save did not go: {error}")));
                None
            }
        }
    }

    /// A merge with no overlap goes into the text with a note; else the dialog of the three
    /// versions opens with this heading.
    fn merge(&mut self, save: &EditSave, remote: &Arc<Remote>, (heading, words): (&str, String)) -> Option<Taken> {
        if remote.merge.is_clean() {
            let text = remote.merge.text(|_| fresnel_core::manifest::merge::Side::Ours);
            let note = format!("{words} Its changes are in the text with yours: check it and save again.");
            self.note = Some((Tone::Warn, note));
            return Some(Taken::Merged(Arc::clone(&remote.read), text));
        }
        self.note = Some((Tone::Warn, words));
        self.conflict = Some(ConflictDialog::new(Arc::clone(remote), save.base.version.as_deref(), heading.to_owned()));
        None
    }
}
