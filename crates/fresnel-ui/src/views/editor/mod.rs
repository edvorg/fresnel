//! The editors of the dock (plan sections 11.1 and 12.9): one tab for each object whose
//! manifest the user edits, with the color of its cluster in its title.
//!
//! The Edit action of the Manifest tab, of a menu, of a key or of the quick search opens an
//! editor. The shell keeps the tabs and names their requests to the core
//! (`Command::ShowEditors`); the core reads the text of each one, and the tab takes it. A
//! save is a dry run, a dialog with its diff, and the replace (`Command::SaveEditor`).
//!
//! - `tab`: one editor. `bar`: its bar and the bar of its search. `find`: the search.
//! - `check`: the check of the text on a worker thread. `marks`: the list of its marks.
//! - `saving`: the saves of a tab. `dialog`: the diff before a save, `diff` its rows.
//!   `conflict`: the three versions of the lines that the server and the user changed.
//! - `create`: a tab that creates or applies a text of documents.
//! - `text`: the text widget with its own virtual rows.

mod bar;
mod check;
mod conflict;
mod create;
mod dialog;
mod diff;
mod find;
mod marks;
mod saving;
mod tab;
mod text;

use egui::{Id, Modal, RichText, Ui};
use fresnel_core::app::{Command, CoreView, EditRequest, EditState};
use fresnel_core::cluster::ClusterId;
use fresnel_core::net::task::Tasks;
use fresnel_core::ops::Target;

use crate::shell::{Scene, Tab, Title};
use crate::widgets::tone::Tone;
use crate::widgets::worker::Wake;
use check::Checker;
use create::CreateTab;
use tab::{EditorTab, TabAsk};

/// The most editors. Each one holds its text, its history and its search.
const MOST: usize = 8;
/// The most tabs of a create.
const MOST_CREATES: usize = 4;

/// The tabs of the editors.
pub(crate) struct EditorTabs {
    tabs: Vec<EditorTab>,
    creates: Vec<CreateTab>,
    /// The identity of the last tab, and the number of the last request.
    last: u64,
    /// The requests that the core got last.
    sent: Vec<EditRequest>,
    checker: Checker,
    /// Why the last editor did not open.
    note: Option<String>,
    /// The tab whose close asks first: its text has changes.
    closing: Option<u64>,
}

impl EditorTabs {
    pub(crate) fn new(tasks: &Tasks, wake: Wake) -> Self {
        let checker = Checker::new(tasks, wake);
        Self { tabs: Vec::new(), creates: Vec::new(), last: 0, sent: Vec::new(), checker, note: None, closing: None }
    }

    /// Opens a tab of a create. With the most such tabs, the first one shows. Gives the
    /// identity of its tab.
    pub(crate) fn open_create(&mut self) -> u64 {
        if self.creates.len() >= MOST_CREATES
            && let Some(first) = self.creates.first()
        {
            return first.id;
        }
        self.last += 1;
        self.creates.push(CreateTab::new(self.last));
        self.last
    }

    /// Opens the editor of an object, or the tab that has it. Gives the identity of its tab.
    /// With the most tabs, the oldest tab with no change closes; when each one has changes,
    /// the oldest one shows, with a note.
    pub(crate) fn open(&mut self, target: Target) -> Option<u64> {
        self.note = None;
        if let Some(tab) = self.tabs.iter().find(|tab| tab.request.target == target) {
            return Some(tab.id);
        }
        if self.tabs.len() >= MOST {
            let Some(index) = self.tabs.iter().position(|tab| !tab.is_changed()) else {
                self.note = Some(format!("Each of the {MOST} editors has changes. Close one to edit an other object."));
                return self.tabs.first().map(|tab| tab.id);
            };
            self.tabs.remove(index);
        }
        self.last += 1;
        self.tabs.push(EditorTab::new(self.last, EditRequest { number: self.last, target }));
        Some(self.last)
    }

    /// Closes a tab. A tab with changes asks first, and gives false.
    pub(crate) fn close(&mut self, id: u64) -> bool {
        let changed = |tab: &CreateTab| tab.id == id && tab.is_changed();
        if self.tabs.iter().any(|tab| tab.id == id && tab.is_changed()) || self.creates.iter().any(changed) {
            self.closing = Some(id);
            return false;
        }
        self.drop_tab(id);
        true
    }

    fn drop_tab(&mut self, id: u64) {
        self.tabs.retain(|tab| tab.id != id);
        self.creates.retain(|tab| tab.id != id);
    }

    pub(crate) fn has(&self, id: u64) -> bool {
        self.tabs.iter().any(|tab| tab.id == id) || self.creates.iter().any(|tab| tab.id == id)
    }

    /// True while a dialog of a save or a create is open: it takes the keys.
    pub(crate) fn has_dialog(&self) -> bool {
        self.tabs.iter().any(EditorTab::has_dialog) || self.creates.iter().any(CreateTab::has_dialog)
    }

    /// The tab whose text or search had the focus of the keys in the last frame, if it is
    /// the open tab of the dock: a hidden tab keeps the flag of its last frame.
    pub(crate) fn focused(&self, open: Tab) -> Option<u64> {
        let Tab::Editor(id) = open else {
            return None;
        };
        let editor = self.tabs.iter().any(|tab| tab.id == id && tab.focused);
        (editor || self.creates.iter().any(|tab| tab.id == id && tab.focused)).then_some(id)
    }

    /// Opens the search of a tab: Cmd+F with the focus in its text.
    pub(crate) fn find(&mut self, id: u64) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
            tab.find();
        }
        if let Some(tab) = self.creates.iter_mut().find(|tab| tab.id == id) {
            tab.find();
        }
    }

    /// The title of each tab, in the sequence of their open.
    pub(crate) fn titles(&self, view: &CoreView) -> Vec<Title> {
        // A tab of a create has the color of its target cluster, and none before it has one.
        let creates = self.creates.iter().map(|tab| {
            let cluster = tab.cluster().cloned().unwrap_or_else(|| ClusterId::new(""));
            let changed = if tab.is_changed() { " •" } else { "" };
            let tone = Tone::Normal;
            (tab.id, Title { tab: Tab::Editor(tab.id), text: format!("YAML{changed}"), cluster, tone, bell: false })
        });
        let mut titles: Vec<(u64, Title)> = self.tabs.iter().map(|tab| (tab.id, title(tab, view))).collect();
        titles.extend(creates);
        titles.sort_by_key(|(id, _)| *id);
        titles.into_iter().map(|(_, title)| title).collect()
    }

    /// Draws the tab `id`, and the dialog of a close.
    pub(crate) fn show(&mut self, id: u64, ui: &mut Ui, scene: &mut Scene<'_>) {
        let note = self.note.as_deref();
        if let Some(tab) = self.creates.iter_mut().find(|tab| tab.id == id) {
            tab.show(ui, scene, &mut self.checker);
        } else if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id)
            && tab.show(ui, scene, &mut self.checker, note) == TabAsk::ReadAgain
        {
            self.last += 1;
            tab.request.number = self.last;
        }
        self.confirm_close(ui.ctx());
    }

    /// The command that names the editors to the core, when they changed.
    pub(crate) fn command(&mut self) -> Option<Command> {
        let requests: Vec<EditRequest> = self.tabs.iter().map(|tab| tab.request.clone()).collect();
        if requests == self.sent {
            return None;
        }
        self.sent.clone_from(&requests);
        Some(Command::ShowEditors(requests))
    }

    /// The dialog of the close of a tab with changes.
    fn confirm_close(&mut self, ctx: &egui::Context) {
        let Some(id) = self.closing else {
            return;
        };
        let editor = self.tabs.iter().find(|tab| tab.id == id).map(|tab| tab::object(&tab.request.target));
        let Some(object) = editor.or_else(|| self.creates.iter().any(|tab| tab.id == id).then(|| "YAML".to_owned()))
        else {
            self.closing = None;
            return;
        };
        let mut close = None;
        Modal::new(Id::new("close editor")).show(ctx, |ui| {
            ui.label(RichText::new(format!("Close the editor of {object}?")).strong());
            ui.label("The text has changes, and the close drops them.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Close and drop the changes").clicked() {
                    close = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    close = Some(false);
                }
            });
        });
        match close {
            Some(true) => {
                self.drop_tab(id);
                self.closing = None;
            }
            Some(false) => self.closing = None,
            None => {}
        }
    }
}

/// "Edit web", "Edit Deployment web ·": the title of a tab, in the tone of its read and its
/// check, with a dot for changes.
fn title(tab: &EditorTab, view: &CoreView) -> Title {
    let target = &tab.request.target;
    let object = match target.kind {
        fresnel_core::kinds::Kind::Pod => target.key.name.to_string(),
        kind => format!("{} {}", kind.info().name, target.key.name),
    };
    let changed = if tab.is_changed() { " •" } else { "" };
    let read = view.edits.iter().find(|read| read.request == tab.request).map(|read| &read.state);
    let tone = match read {
        Some(EditState::Failed(_)) => Tone::Error,
        _ if tab.has_error() => Tone::Error,
        _ => Tone::Normal,
    };
    let text = format!("Edit {}{changed}", crate::format::short(&object, 40));
    Title { tab: Tab::Editor(tab.id), text, cluster: target.cluster.clone(), tone, bell: false }
}
