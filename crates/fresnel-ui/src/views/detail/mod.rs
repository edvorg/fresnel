//! The detail panel of one object (plan section 11.2): a header with the kind, the name,
//! the state and the cluster of the object, and the tabs Overview, Manifest, and Metrics
//! for a kind with metrics (`views/metrics`). Overview
//! has the metadata with the owners as links, the sections of the kind, the conditions,
//! the related objects with links from the indexes, and the Events. A Secret and a ConfigMap
//! have the section of their keys (`views/keys`) after the metadata.
//!
//! The panel reads its object from the snapshot of each frame, and builds its data again
//! for each new version of the object. In the UI prototype, the panel built its data one
//! time and showed a deleted object with each button. Here an object that left a store
//! with a complete list shows as deleted, with its last state.
//!
//! - `fields`: the parts of a section. `lines`: the lines, tables and texts of a kind.
//! - `metadata`: the first section of each kind.
//! - `sections`: the sections of a kind.
//! - `conditions`, `related`, `events`: the sections of each kind. `manifest`: the tab.
//! - `status`: the JSON of the status of an object, for its conditions.
//! - `actions`: the actions of the command table on the object of the panel.

pub(crate) mod actions;
mod conditions;
mod events;
pub(crate) mod fields;
pub(crate) mod lines;
mod manifest;
mod metadata;
pub(crate) mod related;
pub(crate) mod sections;
mod status;

use std::sync::Arc;

use egui::{Align, Button, Frame, Label, Layout, Margin, RichText, ScrollArea, Ui};
use fresnel_core::app::{ClusterRow, CoreView, ManifestRequest};
use fresnel_core::keys::{self, KeyKind};
use fresnel_core::kinds::{Action, Describe, Object};
use fresnel_core::ops::Target;
use fresnel_core::store::Snapshot;

use self::conditions::Condition;
use self::fields::VALUE_LIMIT;
use self::manifest::ManifestTab;
use self::sections::{Context, Pick};
use crate::format::short;
use crate::shell::Scene;
use crate::theme::{self, Palette, SMALL};
use crate::views::act::{Ask, Facts, Gate, Permits, Subject, facts};
use crate::views::keys::KeyPanel;
use crate::views::metrics::has_columns;
use crate::views::metrics::tab::MetricsTab;
use crate::views::objects::{Link, Nouns, old_since, uid};
use crate::widgets::table::cells::ring;
use crate::widgets::table::model::ObjectId;
use crate::widgets::tone::Tone;
use crate::widgets::{self, dot};

/// What the user did in the panel in one frame.
pub(crate) enum Act {
    None,
    Close,
    /// A click on a link.
    Open(Link),
    /// A click on an action.
    Ask(Ask),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Overview,
    Manifest,
    Metrics,
}

/// What the store of the cluster knows of the object of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Presence {
    Present,
    /// The store had a complete list, and the object is not in it.
    Deleted,
    /// The store has no complete list now, so it does not know the object. That is not a
    /// deleted object (design rule 5).
    NotLoaded,
}

/// The panel of one object, with its last state.
pub(crate) struct DetailPanel<K: Describe> {
    id: ObjectId,
    object: Arc<K>,
    presence: Presence,
    /// The conditions of the version of the object, and their count.
    conditions: (Vec<Condition>, usize),
    tab: Tab,
    manifest: ManifestTab,
    metrics: MetricsTab,
    /// The keys of a Secret or a ConfigMap. `None` for each other kind.
    keys: Option<KeyPanel>,
    /// What the actions read of the version of the object.
    facts: Facts,
    /// What the panel asked of the permissions of the user.
    permits: Permits,
}

impl<K: Describe> DetailPanel<K> {
    pub(crate) fn new(id: ObjectId, object: Arc<K>) -> Self {
        let (conditions, facts) = (conditions::of(&*object), facts::of(&*object));
        let (manifest, permits) = (ManifestTab::default(), Permits::default());
        let keys = KeyKind::of(K::ID).map(|kind| {
            let (cluster, key, uid) = (id.cluster.clone(), id.key.clone(), Arc::clone(&id.uid));
            KeyPanel::new(keys::Target { cluster, kind, key, uid })
        });
        let (tab, metrics) = (Tab::Overview, MetricsTab::default());
        Self { id, object, presence: Presence::Present, conditions, tab, manifest, metrics, keys, facts, permits }
    }

    pub(crate) fn id(&self) -> &ObjectId {
        &self.id
    }

    /// The object whose values the core reads for the panel: a Secret or a ConfigMap.
    pub(crate) fn key_target(&self) -> Option<&keys::Target> {
        self.keys.as_ref().map(KeyPanel::target)
    }

    /// True while the dialog of a save of the keys is open.
    pub(crate) fn has_dialog(&self) -> bool {
        self.keys.as_ref().is_some_and(KeyPanel::has_dialog)
    }

    /// True while the store of the cluster has the object.
    pub(crate) fn is_present(&self) -> bool {
        self.presence == Presence::Present
    }

    /// Runs an action that needs no job and no dialog: the Manifest tab, the copy of the name
    /// and the keys of the key view. Gives a note. Gives back each other action: the page
    /// starts it.
    pub(crate) fn act(&mut self, ctx: &egui::Context, action: Action) -> Result<Option<String>, Action> {
        let key = &self.id.key;
        match action {
            Action::ViewManifest => {
                self.tab = Tab::Manifest;
                Ok(None)
            }
            Action::CopyName => {
                ctx.copy_text(key.name.to_string());
                Ok(Some(format!("Copied the name {}.", key.name)))
            }
            Action::EditKeys | Action::AddKey | Action::RemoveKey if self.keys.is_some() => {
                self.tab = Tab::Overview;
                Ok(self.keys.as_mut().and_then(|keys| keys.act(action)))
            }
            other => Err(other),
        }
    }

    /// The object of the panel as the object of an action.
    pub(crate) fn subject(&self) -> Subject {
        let target = Target {
            cluster: self.id.cluster.clone(),
            kind: K::ID,
            key: self.id.key.clone(),
            uid: Arc::clone(&self.id.uid),
        };
        Subject { target, facts: self.facts.clone() }
    }

    /// The gate of an action on the object: an object that left its store has only the
    /// actions that need no object (plan section 11.2).
    pub(crate) fn gate(&mut self, action: Action, scene: &mut Scene<'_>) -> Gate {
        if !self.is_present() && !actions::works_on_gone(action) {
            let text = format!("The {} is not in its store now: its actions are off.", K::ID.info().name);
            return Gate::Closed(text);
        }
        let target = self.subject().target;
        let permits = &mut self.permits;
        Gate::of(action, &self.facts, || permits.decide(scene.view, &target, action, scene.commands))
    }

    /// Reads the object from the snapshot of this frame. An object with the same name and
    /// an other UID is an other object.
    pub(crate) fn refresh(&mut self, snapshot: &Snapshot<K>) {
        let found = snapshot.items.get(&self.id.key).filter(|object| uid(object.meta()) == &*self.id.uid);
        self.presence = match found {
            Some(object) => {
                if !Arc::ptr_eq(object, &self.object) {
                    (self.conditions, self.facts) = (conditions::of(&**object), facts::of(&**object));
                    self.object = Arc::clone(object);
                }
                Presence::Present
            }
            None if snapshot.listed => Presence::Deleted,
            None => Presence::NotLoaded,
        };
    }

    /// Draws the panel. `cluster` is the cluster of the object, with its data.
    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        scene: &mut Scene<'_>,
        cluster: Option<&ClusterRow>,
        nouns: &Nouns,
    ) -> Act {
        let palette = theme::palette(ui);
        let mut act = Act::None;
        Frame::new().inner_margin(Margin { left: 14, right: 12, top: 8, bottom: 6 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(K::ID.info().name).color(palette.text_muted));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let button = ui.add(Button::new(RichText::new("Close").size(SMALL)));
                    widgets::name(&button, "Close the details");
                    if button.clicked() {
                        act = Act::Close;
                    }
                });
            });
            let name = short(&self.id.key.name, VALUE_LIMIT);
            ui.add(Label::new(RichText::new(name).heading().strong()).truncate());
            let data = cluster.and_then(|row| row.data.as_ref());
            let store = data.and_then(|data| data.snapshot::<K>());
            let old = store.as_ref().and_then(|store| old_since(store, scene.clock));
            self.state_line(ui, palette, scene, nouns, old.as_deref());
            // A write of a job, or the last save of the keys.
            let written = self.keys.as_ref().and_then(|keys| keys.written(data));
            let saved = written.is_some_and(|written| store.as_ref().is_some_and(|store| store.is_pending(&written)));
            if saved || store.is_some_and(|store| pending(scene.view, &self.id, &store)) {
                let text = "Change pending: the server stored a change, and the watch did not bring it yet.";
                ui.label(RichText::new(text).color(palette.info));
            }
            ui.add_space(4.0);
            let view = scene.view;
            if let Some(action) = actions::bar(ui, K::ID, &view.config.keys, |action| self.gate(action, scene)) {
                act = Act::Ask(Ask { action, revision: None });
            }
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Overview, "Overview");
                ui.selectable_value(&mut self.tab, Tab::Manifest, "Manifest");
                if has_columns(K::ID) {
                    ui.selectable_value(&mut self.tab, Tab::Metrics, "Metrics");
                }
            });
        });
        let line = ui.available_rect_before_wrap();
        ui.painter().hline(line.x_range(), line.top(), (1.0, palette.border));
        let inner = Margin { left: 14, right: 14, top: 10, bottom: 12 };
        match self.tab {
            Tab::Overview => {
                ScrollArea::vertical().id_salt("detail").auto_shrink(false).show(ui, |ui| {
                    Frame::new().inner_margin(inner).show(ui, |ui| match self.overview(ui, palette, scene, cluster) {
                        Some(Pick::Open(link)) => act = Act::Open(link),
                        Some(Pick::Ask(ask)) => act = Act::Ask(ask),
                        None => {}
                    });
                });
            }
            Tab::Manifest => {
                Frame::new().inner_margin(inner).show(ui, |ui| self.manifest_tab(ui, palette, scene));
            }
            Tab::Metrics => {
                let data = cluster.and_then(|row| row.data.as_ref());
                let cx = Context { palette, view: scene.view, clock: scene.clock, id: &self.id, data };
                ScrollArea::vertical().id_salt("metrics").auto_shrink(false).show(ui, |ui| {
                    Frame::new().inner_margin(inner).show(ui, |ui| self.metrics.show(ui, &cx, &*self.object));
                });
            }
        }
        act
    }

    /// The state of the object, and how far the panel can know it.
    fn state_line(&self, ui: &mut Ui, palette: &Palette, scene: &Scene<'_>, nouns: &Nouns, old: Option<&str>) {
        let (text, health) = sections::state(&*self.object);
        let current = self.presence == Presence::Present && old.is_none();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            // The color of a state is for a state of now.
            let tone = if current { Tone::of_object(health) } else { Tone::Faint };
            dot(ui, tone.color(palette));
            ui.label(RichText::new(short(&text, VALUE_LIMIT)).color(tone.color(palette)));
            ui.label(RichText::new("·").color(palette.text_faint));
            dot(ui, scene.colors.color(&self.id.cluster, palette));
            ui.label(RichText::new(self.id.cluster.as_str()).color(palette.text_muted));
        });
        let (one, many, cluster) = (nouns.one, nouns.many, &self.id.cluster);
        match (self.presence, old) {
            (Presence::Deleted, _) => {
                let text = format!("This {one} is deleted. The panel shows its last state.");
                ui.label(RichText::new(text).color(palette.warn));
            }
            (Presence::NotLoaded, _) => {
                let text = format!(
                    "The {many} of {cluster} are not loaded now. The panel shows the last state of this {one}."
                );
                ui.label(RichText::new(text).color(palette.warn));
            }
            (Presence::Present, Some(old)) => {
                ui.horizontal(|ui| {
                    ring(ui, palette);
                    ui.label(RichText::new(format!("The values are {old}.")).color(palette.warn));
                });
            }
            (Presence::Present, None) => {}
        }
    }

    /// The tab Overview: the metadata, the keys of a Secret or a ConfigMap, the details of the
    /// kind, the conditions, the related objects, the parts of the kind and the Events. Gives
    /// what the user clicked.
    fn overview(
        &mut self,
        ui: &mut Ui,
        palette: &Palette,
        scene: &mut Scene<'_>,
        cluster: Option<&ClusterRow>,
    ) -> Option<Pick> {
        let data = cluster.and_then(|row| row.data.as_ref());
        let mut picked = metadata::show(ui, palette, scene.clock, &self.id, &*self.object).map(Pick::Open);
        ui.add_space(14.0);
        if self.keys.is_some() {
            // The keys pass the gate of each action of the panel.
            let gate = self.gate(Action::EditKeys, scene);
            if let Some(keys) = &mut self.keys {
                keys.show(ui, palette, scene, data, &gate);
            }
            ui.add_space(14.0);
        }
        let cx = Context { palette, view: scene.view, clock: scene.clock, id: &self.id, data };
        picked = sections::details(ui, &cx, &*self.object).or(picked);
        let (conditions, total) = &self.conditions;
        conditions::show(ui, palette, scene.clock, conditions, *total);
        if let Some(data) = data {
            picked = related::show(ui, palette, data, &self.id, &*self.object).map(Pick::Open).or(picked);
        }
        picked = sections::parts(ui, &cx, &*self.object).or(picked);
        events::show(ui, palette, scene.clock, &events::Source { kind: K::ID, id: &self.id, data });
        picked
    }

    fn manifest_tab(&mut self, ui: &mut Ui, palette: &Palette, scene: &mut Scene<'_>) {
        // The UID keeps the manifest of a new object with the same name out of this panel.
        let request = ManifestRequest {
            cluster: self.id.cluster.clone(),
            kind: K::ID,
            key: self.id.key.clone(),
            uid: Arc::clone(&self.id.uid),
            managed_fields: self.manifest.managed_fields(),
        };
        let present = self.presence == Presence::Present;
        let version = present.then(|| self.object.meta().resource_version.as_deref().unwrap_or_default());
        self.manifest.show(ui, palette, scene, manifest::Target { request, version });
    }
}

/// True while a job of this object wrote a change that its store does not show yet (plan
/// section 9.10).
fn pending<K: Describe>(view: &CoreView, id: &ObjectId, store: &Snapshot<K>) -> bool {
    view.jobs.jobs.iter().any(|job| {
        let target = &job.order.target;
        let same = target.cluster == id.cluster && target.kind == K::ID && target.key == id.key;
        same && job.report.outcome.written().is_some_and(|written| store.is_pending(written))
    })
}
