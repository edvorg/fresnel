//! One tab of the editor: the read of its object, then its bar, its text and the marks of
//! the check.
//!
//! The text belongs to the tab from its read on: a later version of the object in its store
//! does not change it. A text of more than 8,000 lines is read-only (plan section 12.9).

use std::sync::Arc;

use egui::{CentralPanel, Frame, Margin, Panel, RichText, Ui};
use fresnel_core::app::{EditRead, EditRequest, EditState};
use fresnel_core::manifest::check::Identity;
use fresnel_core::manifest::{EditText, LINE_LIMIT};
use fresnel_core::ops::Target;

use super::bar::{self, Asked};
use super::check::{Checker, TabCheck};
use super::find::Find;
use super::marks;
use super::text::{Act, Overlay, TextArea};
use crate::shell::Scene;
use crate::theme::{self, PAD};
use crate::widgets::tone::Tone;

/// The state of one tab.
pub(super) struct EditorTab {
    pub(super) id: u64,
    pub(super) request: EditRequest,
    loaded: Option<Loaded>,
    find: Find,
    check: TabCheck,
    /// True when the text or the field of the search had the focus of the keys in the last frame.
    pub(super) focused: bool,
}

/// The text of a tab after its read.
struct Loaded {
    text: TextArea,
    read: Arc<EditText>,
    /// True when the text differs from the read, for a generation of the text.
    changed: (u64, bool),
}

/// What the tab asks of the tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TabAsk {
    None,
    /// A read again after a failed one.
    ReadAgain,
}

impl EditorTab {
    pub(super) fn new(id: u64, request: EditRequest) -> Self {
        Self { id, request, loaded: None, find: Find::default(), check: TabCheck::default(), focused: false }
    }

    /// True when the text differs from its read.
    pub(super) fn is_changed(&self) -> bool {
        self.loaded.as_ref().is_some_and(|loaded| loaded.changed.1)
    }

    /// True when the last check found an error in the text.
    pub(super) fn has_error(&self) -> bool {
        self.check.done.as_ref().is_some_and(|(_, check)| check.has_error())
    }

    /// Opens the search, for Cmd+F.
    pub(super) fn find(&mut self) {
        self.find.start();
    }

    pub(super) fn show(&mut self, ui: &mut Ui, scene: &Scene<'_>, checker: &mut Checker, note: Option<&str>) -> TabAsk {
        let read = scene.view.edits.iter().find(|read| read.request == self.request);
        if self.loaded.is_none()
            && let Some(EditState::Done(text)) = read.map(|read| &read.state)
        {
            let editable = text.lines <= LINE_LIMIT;
            let mut area = TextArea::new(&text.text, LINE_LIMIT, editable);
            area.focus();
            self.loaded = Some(Loaded { text: area, read: Arc::clone(text), changed: (0, false) });
        }
        let margin = Margin { left: PAD as i8, right: PAD as i8, top: 4, bottom: 2 };
        let Some(loaded) = &mut self.loaded else {
            return Frame::new().inner_margin(margin).show(ui, |ui| reading(ui, &self.request.target, read)).inner;
        };
        let generation = loaded.text.generation();
        if loaded.changed.0 != generation {
            loaded.changed = (generation, loaded.text.text().as_str() != &*loaded.read.text);
        }
        if loaded.text.editable() {
            let identity = identity(&self.request.target);
            checker.follow((self.id, &mut self.check), generation, || (loaded.text.text(), identity));
        }
        self.find.update(&loaded.text);
        let check = self.check.done.as_ref().map(|(_, check)| Arc::clone(check));
        let object = object(&self.request.target);
        let mut parts = bar::Parts {
            object: &object,
            read: &loaded.read,
            lines: loaded.text.lines(),
            editable: loaded.text.editable(),
            changed: loaded.changed.1,
            undo: loaded.text.can_undo(),
            redo: loaded.text.can_redo(),
            check: loaded.text.editable().then(|| state(&self.check, generation, checker.failure())),
            notes: bar::notes(&loaded.read, loaded.text.editable()),
        };
        let refusals = loaded.text.note().into_iter().chain(note);
        parts.notes.extend(refusals.map(|note| (Tone::Warn, note.to_owned())));
        let asked = Frame::new().inner_margin(margin).show(ui, |ui| bar::show(ui, &parts, &mut self.find)).inner;
        let now = ui.input(|input| input.time);
        for ask in asked {
            match ask {
                Asked::Undo => loaded.text.act(Act::Undo, now),
                Asked::Redo => loaded.text.act(Act::Redo, now),
                Asked::Revert => {
                    loaded.text.act(Act::SelectAll, now);
                    loaded.text.act(Act::Insert(loaded.read.text.to_string()), now);
                }
                Asked::Go(forward) => self.find.go(&mut loaded.text, forward),
                Asked::Text => loaded.text.focus(),
            }
        }
        if self.focused {
            let (hit, mark) = bar::keys(ui);
            if let Some(forward) = hit {
                self.find.go(&mut loaded.text, forward);
            }
            let cursor = loaded.text.cursor();
            let next = mark.and_then(|forward| check.as_deref().and_then(|check| marks::next(check, cursor, forward)));
            if let Some(mark) = next {
                marks::go_to(&mut loaded.text, mark);
            }
        }
        // The marks take the bottom and the text the rest, so the tab is as high as the dock.
        if let Some(check) = check.as_deref().filter(|check| !check.marks.is_empty()) {
            let panel = Panel::bottom(ui.id().with("editor marks")).resizable(false);
            let chosen = panel.frame(Frame::new().inner_margin(margin)).show(ui, |ui| marks::show(ui, check)).inner;
            if let Some(mark) = chosen {
                marks::go_to(&mut loaded.text, mark);
            }
        }
        let overlay = Overlay {
            marks: check.as_deref().map_or(&[], |check| check.marks.as_slice()),
            hits: self.find.hits(),
            current: self.find.current(),
        };
        CentralPanel::default().frame(Frame::NONE).show(ui, |ui| loaded.text.show(ui, overlay));
        let field = ui.memory(|memory| memory.focused().is_some_and(|id| self.find.owns(id)));
        self.focused = loaded.text.has_focus() || field;
        TabAsk::None
    }
}

/// The note of a tab before its text: the read, or why it failed, with a button that reads again.
fn reading(ui: &mut Ui, target: &Target, read: Option<&Arc<EditRead>>) -> TabAsk {
    let palette = theme::palette(ui);
    match read.map(|read| &read.state) {
        Some(EditState::Failed(error)) => {
            ui.label(RichText::new(format!("The read failed: {error}")).color(palette.error));
            if ui.button("Read again").clicked() {
                return TabAsk::ReadAgain;
            }
        }
        _ => {
            ui.label(RichText::new(format!("The manifest is read from {}…", target.cluster)).color(palette.text_muted));
        }
    }
    ui.allocate_space(ui.available_size());
    TabAsk::None
}

/// The words of the state of the check, with true for an error.
fn state(check: &TabCheck, generation: u64, failure: Option<String>) -> (String, bool) {
    if let Some(failure) = failure {
        return (format!("The check stopped: {failure}"), true);
    }
    match &check.done {
        Some((_, done)) if !check.is_behind(generation) => (marks::summary(done), done.has_error()),
        _ => ("Checking…".to_owned(), false),
    }
}

/// "Deployment shop/web in ams-prod": the object of a tab.
pub(super) fn object(target: &Target) -> String {
    let key = &target.key;
    let name = if key.namespace.is_empty() { key.name.to_string() } else { format!("{}/{}", key.namespace, key.name) };
    format!("{} {name} in {}", target.kind.info().name, target.cluster)
}

/// The object that the text of a tab must stay.
fn identity(target: &Target) -> Identity {
    Identity { kind: target.kind, key: target.key.clone() }
}
