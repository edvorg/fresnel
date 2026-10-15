//! The metadata of the object of the panel: its namespace, cluster, UID, times, owners as
//! links, labels and annotations.

use std::time::SystemTime;

use egui::{RichText, Ui};
use fresnel_core::kinds::{Describe, Kind, Object};
use fresnel_core::store::ObjectKey;

use super::fields::{self, NONE, VALUE_LIMIT, chips, field, heading, text_field};
use super::related;
use crate::format::{self, Clock, short};
use crate::theme::Palette;
use crate::views::objects::Link;
use crate::views::pod_spec;
use crate::widgets::table::model::ObjectId;

/// The metadata, with each owner as a link. Gives the owner that the user clicked.
pub(super) fn show<K: Describe>(
    ui: &mut Ui,
    palette: &Palette,
    clock: &Clock,
    id: &ObjectId,
    object: &K,
) -> Option<Link> {
    let meta = object.meta();
    heading(ui, palette, "METADATA");
    if K::ID.info().namespaced {
        text_field(ui, palette, "Namespace", &id.key.namespace);
    }
    text_field(ui, palette, "Cluster", id.cluster.as_str());
    text_field(ui, palette, "UID", &id.uid);
    let created = meta.creation_timestamp.as_ref().map(|time| {
        let at = SystemTime::from(time.0);
        format!("{} ({} ago)", clock.date_time(at), format::span(clock.since(at)))
    });
    text_field(ui, palette, "Created", created.as_deref().unwrap_or(NONE));
    // An object that stays in its delete waits for its finalizers (research 08 section 4.8).
    if let Some(time) = &meta.deletion_timestamp {
        text_field(ui, palette, "Deleting since", &pod_spec::texts::at(time, clock));
        let finalizers = meta.finalizers.as_deref().unwrap_or_default();
        let text = if finalizers.is_empty() { NONE.to_owned() } else { finalizers.join(", ") };
        text_field(ui, palette, "Finalizers", &text);
    }
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
                    let namespace = if kind.info().namespaced { &*id.key.namespace } else { "" };
                    let key = ObjectKey::new(namespace, &owner.name);
                    clicked = Some(Link { cluster: id.cluster.clone(), kind, key });
                }
            }
        });
    });
    field(ui, palette, "Labels", |ui| chips(ui, palette, meta.labels.as_ref()));
    field(ui, palette, "Annotations", |ui| chips(ui, palette, meta.annotations.as_ref()));
    clicked
}
