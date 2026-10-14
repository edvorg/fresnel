//! The detail panel of one object (plan section 11.2): a header with the kind, the name,
//! the state and the cluster of the object, and the tabs Overview and Manifest. Overview
//! has the metadata with the owners as links, the sections of the kind, the conditions,
//! the related objects with links from the indexes, and the Events.
//!
//! The panel reads its object from the snapshot of each frame, and builds its data again
//! for each new version of the object. In the UI prototype, the panel built its data one
//! time and showed a deleted object with each button. Here an object that left a store
//! with a complete list shows as deleted, with its last state.
//!
//! - `fields`: the parts of a section. `sections`: the sections of a kind.
//! - `conditions`, `related`, `events`: the sections of each kind. `manifest`: the tab.

mod conditions;
mod events;
pub(crate) mod fields;
mod manifest;
mod related;
mod sections;

use std::sync::Arc;
use std::time::SystemTime;

use egui::{Align, Button, Frame, Label, Layout, Margin, RichText, ScrollArea, Ui};
use fresnel_core::app::{ClusterRow, ManifestRequest};
use fresnel_core::kinds::{Describe, Kind, Object};
use fresnel_core::store::{ObjectKey, Snapshot};

use self::conditions::Condition;
use self::fields::{NONE, VALUE_LIMIT, chips, field, heading, text_field};
use self::manifest::ManifestTab;
use crate::format::{self, short};
use crate::shell::Scene;
use crate::theme::{self, Palette, SMALL};
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Overview,
    Manifest,
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
}

impl<K: Describe> DetailPanel<K> {
    pub(crate) fn new(id: ObjectId, object: Arc<K>) -> Self {
        let conditions = conditions::of(&*object);
        let manifest = ManifestTab::default();
        Self { id, object, presence: Presence::Present, conditions, tab: Tab::Overview, manifest }
    }

    pub(crate) fn id(&self) -> &ObjectId {
        &self.id
    }

    /// Reads the object from the snapshot of this frame. An object with the same name and
    /// an other UID is an other object.
    pub(crate) fn refresh(&mut self, snapshot: &Snapshot<K>) {
        let found = snapshot.items.get(&self.id.key).filter(|object| uid(object.meta()) == &*self.id.uid);
        self.presence = match found {
            Some(object) => {
                if !Arc::ptr_eq(object, &self.object) {
                    self.conditions = conditions::of(&**object);
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
            let old = data.and_then(|data| data.snapshot::<K>()).and_then(|store| old_since(&store, scene.clock));
            self.state_line(ui, palette, scene, nouns, old.as_deref());
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Overview, "Overview");
                ui.selectable_value(&mut self.tab, Tab::Manifest, "Manifest");
            });
        });
        let line = ui.available_rect_before_wrap();
        ui.painter().hline(line.x_range(), line.top(), (1.0, palette.border));
        let inner = Margin { left: 14, right: 14, top: 10, bottom: 12 };
        match self.tab {
            Tab::Overview => {
                ScrollArea::vertical().id_salt("detail").auto_shrink(false).show(ui, |ui| {
                    Frame::new().inner_margin(inner).show(ui, |ui| {
                        if let Some(link) = self.overview(ui, palette, scene, cluster) {
                            act = Act::Open(link);
                        }
                    });
                });
            }
            Tab::Manifest => {
                Frame::new().inner_margin(inner).show(ui, |ui| self.manifest_tab(ui, palette, scene));
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

    /// The tab Overview. Gives the object of a link that the user clicked.
    fn overview(
        &self,
        ui: &mut Ui,
        palette: &Palette,
        scene: &Scene<'_>,
        cluster: Option<&ClusterRow>,
    ) -> Option<Link> {
        let mut clicked = self.metadata(ui, palette, scene);
        ui.add_space(14.0);
        let (conditions, total) = &self.conditions;
        conditions::show(ui, palette, scene.clock, conditions, *total);
        let data = cluster.and_then(|row| row.data.as_ref());
        if let Some(data) = data {
            clicked = related::show(ui, palette, data, &self.id, &*self.object).or(clicked);
        }
        events::show(ui, palette, scene.clock, &events::Source { kind: K::ID, id: &self.id, data });
        clicked
    }

    /// The metadata, with each owner as a link. Gives the owner that the user clicked.
    fn metadata(&self, ui: &mut Ui, palette: &Palette, scene: &Scene<'_>) -> Option<Link> {
        let meta = self.object.meta();
        heading(ui, palette, "METADATA");
        if K::ID.info().namespaced {
            text_field(ui, palette, "Namespace", &self.id.key.namespace);
        }
        text_field(ui, palette, "Cluster", self.id.cluster.as_str());
        text_field(ui, palette, "UID", &self.id.uid);
        let created = meta.creation_timestamp.as_ref().map(|time| {
            let at = SystemTime::from(time.0);
            format!("{} ({} ago)", scene.clock.date_time(at), format::span(scene.clock.since(at)))
        });
        text_field(ui, palette, "Created", created.as_deref().unwrap_or(NONE));
        sections::fields(ui, palette, &*self.object);
        let mut clicked = None;
        field(ui, palette, "Controlled by", |ui| {
            let owners = meta.owner_references.iter().flatten();
            if meta.owner_references.as_ref().is_none_or(Vec::is_empty) {
                ui.label(RichText::new(NONE).color(palette.text_faint));
            }
            ui.horizontal_wrapped(|ui| {
                related::links_only(ui);
                for owner in owners.take(fields::ITEM_LIMIT) {
                    let text = short(&format!("{} {}", owner.kind, owner.name), VALUE_LIMIT).into_owned();
                    let Some(kind) = Kind::named(&owner.kind) else {
                        ui.label(text);
                        continue;
                    };
                    if ui.link(text).clicked() {
                        let namespace = if kind.info().namespaced { &*self.id.key.namespace } else { "" };
                        let key = ObjectKey::new(namespace, &owner.name);
                        clicked = Some(Link { cluster: self.id.cluster.clone(), kind, key });
                    }
                }
            });
        });
        field(ui, palette, "Labels", |ui| chips(ui, palette, meta.labels.as_ref()));
        field(ui, palette, "Annotations", |ui| chips(ui, palette, meta.annotations.as_ref()));
        clicked
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
