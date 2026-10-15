//! The sections of a Namespace (`research/01` section 3.4.1, `research/02` section 4.5): its
//! phase and its finalizers; then each ResourceQuota with its use and each LimitRange with
//! its limits, from the stores of their full objects that the page of the Namespaces holds
//! (`Command::ShowKinds`); then the count of the objects of each kind in the namespace.
//!
//! The counts come from the store of each kind that is always on, with no request. A count
//! of a store that is not live has its mark: it can be old.

use egui::{RichText, Ui};
use fresnel_core::app::ClusterData;
use fresnel_core::kinds::{Describe, Kind};
use fresnel_core::store::Snapshot;
use k8s_openapi::api::core::v1::{LimitRange, Namespace, ResourceQuota};

use crate::format;
use crate::views::config::quota::{limits, usage};
use crate::views::detail::fields::{ITEM_LIMIT, heading, more, note};
use crate::views::detail::lines::{details_heading, line, none, table};
use crate::views::detail::sections::Context;
use crate::views::objects::{Link, old_since, old_text};

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, namespace: &Namespace) -> Option<Link> {
    details_heading(ui, cx);
    let phase = namespace.status.as_ref().and_then(|status| status.phase.clone());
    line(ui, cx, "Status", Some(phase.unwrap_or_else(|| "Active".to_owned())));
    let finalizers = namespace.spec.as_ref().and_then(|spec| spec.finalizers.as_deref()).unwrap_or_default();
    if !finalizers.is_empty() {
        line(ui, cx, "Finalizers", Some(finalizers.join(", ")));
    }
    ui.add_space(10.0);
    None
}

/// The quotas, the limit ranges and the counts of the objects.
pub(crate) fn parts(ui: &mut Ui, cx: &Context<'_>) -> Option<Link> {
    let Some(data) = cx.data else {
        note(ui, cx.palette.warn, "The objects of the namespace are not loaded: the cluster is not connected.");
        ui.add_space(10.0);
        return None;
    };
    let mut clicked = None;
    heading(ui, cx.palette, "RESOURCE QUOTAS");
    for_each::<ResourceQuota>(ui, cx, data, |ui, quota| {
        clicked = title(ui, cx, Kind::ResourceQuota, &quota.metadata.name).or(clicked.take());
        usage(ui, cx, quota);
    });
    heading(ui, cx.palette, "LIMIT RANGES");
    for_each::<LimitRange>(ui, cx, data, |ui, range| {
        clicked = title(ui, cx, Kind::LimitRange, &range.metadata.name).or(clicked.take());
        limits(ui, cx, range);
    });
    heading(ui, cx.palette, "OBJECTS");
    counts(ui, cx, data);
    ui.add_space(10.0);
    clicked
}

/// Draws each object of a kind in the namespace, with a note when the store of the kind
/// is not current.
fn for_each<K: Describe>(ui: &mut Ui, cx: &Context<'_>, data: &ClusterData, mut draw: impl FnMut(&mut Ui, &K)) {
    let name = &cx.id.key.name;
    let Some(snapshot) = data.snapshot::<K>() else {
        note(ui, cx.palette.warn, &format!("The {} are not loaded: their store starts.", K::TITLE));
        ui.add_space(10.0);
        return;
    };
    notes(ui, cx, &snapshot);
    let found: Vec<&K> =
        snapshot.items.iter().filter(|(key, _)| key.namespace == *name).map(|(_, object)| &**object).collect();
    if found.is_empty() && snapshot.listed {
        none(ui, cx);
    }
    for object in found.iter().take(ITEM_LIMIT) {
        draw(ui, object);
        ui.add_space(6.0);
    }
    more(ui, cx.palette, found.len());
    ui.add_space(10.0);
}

/// The note of a snapshot that is not current.
fn notes<K: Describe>(ui: &mut Ui, cx: &Context<'_>, snapshot: &Snapshot<K>) {
    if !snapshot.listed {
        note(ui, cx.palette.warn, &format!("The {} are not loaded: their store {}.", K::TITLE, snapshot.state));
    } else if let Some(old) = old_since(snapshot, cx.clock) {
        note(ui, cx.palette.warn, &format!("The {} are {old}.", K::TITLE));
    }
}

/// The name of a quota or of a limit range as a link to its panel.
fn title(ui: &mut Ui, cx: &Context<'_>, kind: Kind, name: &Option<String>) -> Option<Link> {
    let name = name.as_deref().unwrap_or_default();
    cx.link(ui, &format!("{} {name}", kind.info().name), kind, &cx.id.key.name, name)
}

/// The count of the objects of each kind with a namespace, from the store that is always
/// on, with the mark of a store that is not live.
fn counts(ui: &mut Ui, cx: &Context<'_>, data: &ClusterData) {
    let name = &cx.id.key.name;
    let rows: Vec<(Kind, usize, Option<String>)> = data
        .sources
        .iter()
        .filter(|source| source.status.kind.info().namespaced)
        .filter_map(|source| {
            let status = &source.status;
            let count = source.count_in(name);
            let mark = match (status.listed, status.state.is_live()) {
                (false, _) => Some(format!("not loaded: the store {}", status.state)),
                (true, false) => Some(old_text(status.last_contact, cx.clock)),
                (true, true) => None,
            };
            (count > 0 || mark.is_some()).then_some((status.kind, count, mark))
        })
        .collect();
    if rows.is_empty() && !data.sources.is_empty() {
        none(ui, cx);
    }
    table(ui, cx, "objects", &["Kind", "Objects", ""], &rows, |ui, (kind, count, mark)| {
        ui.label(kind.info().title);
        ui.label(format::thousands(*count as u64));
        ui.label(RichText::new(mark.as_deref().unwrap_or_default()).color(cx.palette.warn));
    });
}
