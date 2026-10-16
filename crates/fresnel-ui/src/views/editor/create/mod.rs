//! A tab that creates or applies a text of documents (plan section 12.9): the text with the
//! colors, the search and the marks of an editor, a target cluster and namespace, and the
//! jobs of its last run.
//!
//! The check of the text gives its documents. Create… and Apply… open a dialog with each
//! document as cluster, namespace, kind and name, and send one `Command::RunInTurn`: each
//! document is a job of the Operations tab, and each job waits for the one before it.
//!
//! - `plan`: the orders of the documents, and the jobs of a run. `bar`: the bar of the tab.
//! - `confirm`: the dialog before a run. `results`: the documents of the last run.

mod bar;
mod confirm;
mod plan;
mod results;

use std::sync::Arc;

use egui::{Frame, Margin, Ui, UiBuilder};
use fresnel_core::app::Command;
use fresnel_core::cluster::ClusterId;
use fresnel_core::manifest::LINE_LIMIT;

use super::bar::{self as editor_bar, Asked};
use super::check::{Checker, TabCheck};
use super::find::Find;
use super::marks;
use super::text::{Act, Overlay, TextArea};
use crate::shell::Scene;
use crate::theme::{self, PAD};
use confirm::Confirm;
use plan::{Run, Way};

/// The state of one tab of a create.
pub(super) struct CreateTab {
    pub(super) id: u64,
    text: TextArea,
    find: Find,
    check: TabCheck,
    cluster: Option<ClusterId>,
    namespace: String,
    /// True until the first frame takes the target from the view.
    fresh: bool,
    dialog: Option<Confirm>,
    run: Option<Run>,
    /// True when the text or the field of the search had the focus of the keys in the last frame.
    pub(super) focused: bool,
}

impl CreateTab {
    pub(super) fn new(id: u64) -> Self {
        let mut text = TextArea::new("", LINE_LIMIT, true);
        text.focus();
        Self {
            id,
            text,
            find: Find::default(),
            check: TabCheck::default(),
            cluster: None,
            namespace: String::new(),
            fresh: true,
            dialog: None,
            run: None,
            focused: false,
        }
    }

    /// The target cluster, for the color of the title.
    pub(super) fn cluster(&self) -> Option<&ClusterId> {
        self.cluster.as_ref()
    }

    /// True when the text has a character: a close drops it. The title asks in each frame,
    /// so this reads no copy of the text.
    pub(super) fn is_changed(&self) -> bool {
        self.text.lines() > 1 || !self.text.line(0).is_empty()
    }

    pub(super) fn has_dialog(&self) -> bool {
        self.dialog.is_some()
    }

    /// Opens the search, for Cmd+F.
    pub(super) fn find(&mut self) {
        self.find.start();
    }

    pub(super) fn show(&mut self, ui: &mut Ui, scene: &mut Scene<'_>, checker: &mut Checker) {
        if std::mem::take(&mut self.fresh) {
            self.namespace = default_namespace(scene);
        }
        // A cluster connects after the open of the tab at the start of the app.
        if self.cluster.is_none() {
            self.cluster = default_cluster(scene);
        }
        let generation = self.text.generation();
        checker.follow((self.id, &mut self.check), generation, || (self.text.text(), None));
        self.find.update(&self.text);
        let done = self.check.done.as_ref().filter(|_| !self.check.is_behind(generation)).map(|(_, done)| done.clone());
        let parts = bar::Parts {
            undo: self.text.can_undo(),
            redo: self.text.can_redo(),
            run: self.gate(scene, done.as_ref()),
            check: match (&done, checker.failure()) {
                (_, Some(failure)) => (format!("The check stopped: {failure}"), true),
                (Some(done), None) => (marks::summary(&done.check), done.check.has_error()),
                (None, None) => ("Checking…".to_owned(), false),
            },
        };
        let margin = Margin { left: PAD as i8, right: PAD as i8, top: 4, bottom: 2 };
        let asked = Frame::new()
            .inner_margin(margin)
            .show(ui, |ui| {
                let mut asked = bar::show(ui, scene, (&mut self.cluster, &mut self.namespace), &parts);
                if self.find.open {
                    let mut text = Vec::new();
                    editor_bar::find_bar(ui, &mut self.find, &mut text);
                    asked.extend(text.into_iter().map(bar::Ask::Text));
                }
                asked
            })
            .inner;
        let now = ui.input(|input| input.time);
        for ask in asked {
            match ask {
                bar::Ask::Text(Asked::Undo) => self.text.act(Act::Undo, now),
                bar::Ask::Text(Asked::Redo) => self.text.act(Act::Redo, now),
                bar::Ask::Text(Asked::Go(forward)) => self.find.go(&mut self.text, forward),
                bar::Ask::Text(Asked::Text | Asked::Revert | Asked::Save) => self.text.focus(),
                bar::Ask::Find if self.find.open => self.find.open = false,
                bar::Ask::Find => self.find.start(),
                bar::Ask::Run(way) => self.confirm(way, done.as_ref()),
            }
        }
        if self.focused {
            let (hit, mark) = editor_bar::keys(ui);
            if let Some(forward) = hit {
                self.find.go(&mut self.text, forward);
            }
            let place = self.text.cursor_place();
            let check = done.as_ref().map(|done| &*done.check);
            if let Some(mark) = mark.and_then(|forward| check.and_then(|check| marks::next(check, place, forward))) {
                marks::go_to(&mut self.text, mark);
            }
        }
        self.dialogs(ui, scene);
        let check = done.map(|done| Arc::clone(&done.check));
        self.body(ui, scene, check.as_deref(), margin);
    }

    /// The text, the marks of its check and the documents of the last run.
    fn body(
        &mut self,
        ui: &mut Ui,
        scene: &Scene<'_>,
        check: Option<&fresnel_core::manifest::check::Check>,
        margin: Margin,
    ) {
        let rest = ui.available_rect_before_wrap();
        let list = check.filter(|check| !check.marks.is_empty());
        let margins = f32::from(margin.top + margin.bottom);
        let list_height = list.map_or(0.0, |check| marks::height(ui, check) + margins);
        let run_height = if self.run.is_some() { rest.height() * 0.35 } else { 0.0 };
        let overlay = Overlay {
            marks: check.map_or(&[], |check| check.marks.as_slice()),
            hits: self.find.hits(),
            current: self.find.current(),
        };
        let text_rect = rest.with_max_y(rest.max.y - list_height - run_height);
        ui.scope_builder(UiBuilder::new().max_rect(text_rect), |ui| self.text.show(ui, overlay));
        if let Some(check) = list {
            let place = rest.with_min_y(text_rect.max.y).with_max_y(text_rect.max.y + list_height);
            let frame = Frame::new().inner_margin(margin);
            let chosen = ui.scope_builder(UiBuilder::new().max_rect(place), |ui| {
                frame.show(ui, |ui| marks::show(ui, check)).inner
            });
            if let Some(mark) = chosen.inner {
                marks::go_to(&mut self.text, mark);
            }
        }
        let force = self.run.as_ref().and_then(|run| {
            let place = UiBuilder::new().max_rect(rest.with_min_y(rest.max.y - run_height));
            ui.scope_builder(place, |ui| {
                Frame::new().inner_margin(margin).show(ui, |ui| results::show(ui, run, scene.view)).inner
            })
            .inner
        });
        if let (Some(items), Some(cluster)) = (force, self.cluster.clone()) {
            self.dialog = Some(Confirm::force((cluster, self.namespace.clone()), items));
        }
        // The tab is as high as the dock: a list of the run that takes less would make the dock
        // lower in each frame.
        ui.advance_cursor_after_rect(rest);
        let field = ui.memory(|memory| memory.focused().is_some_and(|id| self.find.owns(id)));
        self.focused = self.text.has_focus() || field;
    }

    /// The dialog of a run, and its command.
    fn dialogs(&mut self, ui: &Ui, scene: &mut Scene<'_>) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let cluster =
            dialog.items.iter().find_map(|item| item.order.as_ref().ok()).map(|order| order.target.cluster.clone());
        let color = cluster
            .or(self.cluster.clone())
            .map_or(theme::palette(ui).text_muted, |cluster| scene.colors.color(&cluster, theme::palette(ui)));
        match dialog.show(ui.ctx(), color, scene.view) {
            Some(confirm::Answer::Run) => {
                let run = Run::new(dialog.way, dialog.items.clone(), scene.view);
                scene.commands.push(Command::RunInTurn(run.orders()));
                self.run = Some(run);
                self.dialog = None;
            }
            Some(confirm::Answer::Cancel) => self.dialog = None,
            None => {}
        }
    }

    /// Opens the dialog of a run of the documents of the check.
    fn confirm(&mut self, way: Way, done: Option<&super::check::Checked>) {
        let documents = done.and_then(|done| done.documents.clone());
        let (Some(cluster), Some(documents)) = (self.cluster.clone(), documents) else {
            return;
        };
        if let Ok(entries) = &*documents {
            let items = plan::plan(entries, (&cluster, self.namespace.trim()), way);
            self.dialog = Some(Confirm::new(way, (cluster, self.namespace.trim().to_owned()), items));
        }
    }

    /// Whether the text can go now, or why not.
    fn gate(&self, scene: &Scene<'_>, done: Option<&super::check::Checked>) -> Result<(), String> {
        let Some(cluster) = &self.cluster else {
            return Err("Choose a connected cluster.".to_owned());
        };
        if let Some(refusal) = scene.view.config.refusal(cluster) {
            return Err(refusal);
        }
        if self.run.as_ref().is_some_and(|run| !results::ended(run, scene.view)) {
            return Err("The jobs of the last run did not end yet.".to_owned());
        }
        let Some(documents) = done.and_then(|done| done.documents.as_deref()) else {
            return Err("The check of the text did not end yet.".to_owned());
        };
        match documents {
            Err(error) => Err(format!("The parser refuses the text: {error}.")),
            Ok(entries) if entries.is_empty() => Err("Paste or type the YAML of one or more objects.".to_owned()),
            Ok(_) => Ok(()),
        }
    }
}

/// The first selected and connected cluster that is not read-only, else the first one.
fn default_cluster(scene: &Scene<'_>) -> Option<ClusterId> {
    let config = &scene.view.config;
    let connected = scene.view.clusters.iter().filter(|row| row.selected && row.data.is_some());
    let mut writable = connected.clone().filter(|row| !config.read_only(&row.id));
    writable.next().or_else(|| connected.clone().next()).map(|row| row.id.clone())
}

/// The one namespace of the filter, else `default`.
fn default_namespace(scene: &Scene<'_>) -> String {
    let mut filter = scene.namespaces.iter();
    match (filter.next(), filter.next()) {
        (Some(only), None) => only.to_string(),
        _ => "default".to_owned(),
    }
}
