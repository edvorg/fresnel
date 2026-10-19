//! One tab of the editor: the read of its object, then its bar, its text and the marks of
//! the check, and its saves.
//!
//! The text belongs to the tab from its read on: a later version of the object in its store
//! does not change it. A save gives the tab a new read: the answer of the server, or the new
//! version of a conflict with the merge. A text of more than 8,000 lines is read-only (plan
//! section 12.9).

use std::sync::Arc;

use egui::{Frame, Margin, RichText, Ui, UiBuilder};
use fresnel_core::app::{EditRead, EditRequest, EditState, Stage};
use fresnel_core::manifest::check::Identity;
use fresnel_core::manifest::{EditText, LINE_LIMIT};
use fresnel_core::ops::Target;

use super::bar::{self, Asked};
use super::check::{Checker, TabCheck};
use super::differ::Differ;
use super::find::Find;
use super::marks;
use super::saving::{Saving, Taken};
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
    saving: Saving,
    /// True when the text or the field of the search had the focus of the keys in the last frame.
    pub(super) focused: bool,
}

/// The text of a tab after its read.
struct Loaded {
    text: TextArea,
    /// The read that the text starts from: the first read, or the new read of a save.
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
        let (find, check, saving) = (Find::default(), TabCheck::default(), Saving::default());
        Self { id, request, loaded: None, find, check, saving, focused: false }
    }

    /// True when the text differs from its read.
    pub(super) fn is_changed(&self) -> bool {
        self.loaded.as_ref().is_some_and(|loaded| loaded.changed.1)
    }

    /// True when the last check found an error in the text.
    pub(super) fn has_error(&self) -> bool {
        self.check.done.as_ref().is_some_and(|(_, done)| done.check.has_error())
    }

    /// Opens the search, for Cmd+F.
    pub(super) fn find(&mut self) {
        self.find.start();
    }

    /// True while a dialog of a save is open.
    pub(super) fn has_dialog(&self) -> bool {
        self.saving.has_dialog()
    }

    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        scene: &mut Scene<'_>,
        (checker, differ): (&mut Checker, &mut Differ),
        note: Option<&str>,
    ) -> TabAsk {
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
        let now = ui.input(|input| input.time);
        let saved = read.and_then(|read| read.save.as_ref());
        let color = scene.colors.color(&self.request.target.cluster, theme::palette(ui));
        let (command, taken) = self.saving.follow(ui.ctx(), (scene.view, saved), color, differ);
        scene.commands.extend(command);
        let pending = self.saving.pending();
        // The answer of a save becomes the text, so the text waits for it: an edit in the
        // meantime would be lost.
        loaded.text.hold(pending.is_some());
        if let Some(taken) = taken {
            take(loaded, taken, now);
            // The title of the tab came before this change in the frame.
            ui.ctx().request_repaint();
        }
        let generation = loaded.text.generation();
        if loaded.changed.0 != generation {
            loaded.changed = (generation, loaded.text.text().as_str() != &*loaded.read.text);
        }
        if loaded.text.editable() {
            let identity = identity(&self.request.target);
            checker.follow((self.id, &mut self.check), generation, || (loaded.text.text(), Some(identity)));
        }
        self.find.update(&loaded.text);
        let check = self.check.done.as_ref().map(|(_, done)| Arc::clone(&done.check));
        let object = object(&self.request.target);
        let refusal = scene.view.config.refusal(&self.request.target.cluster);
        let save = match (refusal, pending, check.as_deref().is_some_and(|check| check.has_error())) {
            (Some(refusal), _, _) => Err(refusal),
            (None, Some(_), _) => Err("The save is on its way.".to_owned()),
            (None, None, _) if !loaded.changed.1 => Err("The text has no change to save.".to_owned()),
            (None, None, true) => Err("The parser refuses the text: correct its error first.".to_owned()),
            (None, None, false) => Ok(()),
        };
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
            save,
        };
        let refusals = loaded.text.note().into_iter().chain(note);
        parts.notes.extend(refusals.map(|note| (Tone::Warn, note.to_owned())));
        let on_way = match pending {
            Some(Stage::Check) => Some((Tone::Info, "The server checks the text in a dry run…".to_owned())),
            Some(Stage::Send) => Some((Tone::Info, "The save is on its way…".to_owned())),
            None => self.saving.note.clone(),
        };
        parts.notes.extend(on_way);
        let asked = Frame::new().inner_margin(margin).show(ui, |ui| bar::show(ui, &parts, &mut self.find)).inner;
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
                Asked::Save => scene.commands.push(self.saving.check(&self.request, &loaded.read, loaded.text.text())),
            }
        }
        if self.focused {
            let (hit, mark) = bar::keys(ui);
            if let Some(forward) = hit {
                self.find.go(&mut loaded.text, forward);
            }
            let place = loaded.text.cursor_place();
            let next = mark.and_then(|forward| check.as_deref().and_then(|check| marks::next(check, place, forward)));
            if let Some(mark) = next {
                marks::go_to(&mut loaded.text, mark);
            }
        }
        // The marks take the bottom and the text the rest: the tab is as high as the dock, and
        // a tab that took more would grow the dock in each frame.
        let rest = ui.available_rect_before_wrap();
        let list = check.as_deref().filter(|check| !check.marks.is_empty());
        let margins = f32::from(margin.top + margin.bottom);
        let list_height = list.map_or(0.0, |check| marks::height(ui, check) + margins);
        let overlay = Overlay {
            marks: check.as_deref().map_or(&[], |check| check.marks.as_slice()),
            hits: self.find.hits(),
            current: self.find.current(),
        };
        let text_rect = rest.with_max_y(rest.max.y - list_height);
        ui.scope_builder(UiBuilder::new().max_rect(text_rect), |ui| loaded.text.show(ui, overlay));
        if let Some(check) = list {
            let place = UiBuilder::new().max_rect(rest.with_min_y(text_rect.max.y));
            let frame = Frame::new().inner_margin(margin);
            let chosen = ui.scope_builder(place, |ui| frame.show(ui, |ui| marks::show(ui, check)).inner).inner;
            if let Some(mark) = chosen {
                marks::go_to(&mut loaded.text, mark);
            }
        }
        let field = ui.memory(|memory| memory.focused().is_some_and(|id| self.find.owns(id)));
        self.focused = loaded.text.has_focus() || field;
        TabAsk::None
    }
}

/// Takes the new read of a save and its text, as one step of the undo, with the cursor in
/// its place.
fn take(loaded: &mut Loaded, taken: Taken, now: f64) {
    let (read, text) = match taken {
        Taken::Saved(answer) => (Arc::clone(&answer), answer.text.to_string()),
        Taken::Merged(read, text) => (read, text),
    };
    let at = loaded.text.cursor();
    loaded.text.hold(false);
    loaded.text.act(Act::SelectAll, now);
    loaded.text.act(Act::Insert(text), now);
    loaded.text.select(at..at);
    loaded.read = read;
    loaded.changed = (u64::MAX, false);
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
        Some((_, done)) if !check.is_behind(generation) => (marks::summary(&done.check), done.check.has_error()),
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
