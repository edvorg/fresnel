//! The objects that relate to the object of the panel (plan sections 9.8 and 9.9), from
//! the indexes of its cluster: the ReplicaSets of a Deployment, the pods behind a Service,
//! the pods that use an object, and what a pod uses. Each one is a link to its panel.
//!
//! Each answer of an index tells what it is good for, and the section shows that next to
//! the links: "no pods" and "the pods are not loaded" never look the same.

use std::any::Any;

use egui::{RichText, Ui};
use fresnel_core::app::ClusterData;
use fresnel_core::index::{Answer, Basis, IndexState, ObjectRef, Target, references_of};
use fresnel_core::kinds::{Describe, Kind};
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::core::v1::Pod;

use super::fields::{ITEM_LIMIT, heading, more, note};
use crate::format::short;
use crate::theme::Palette;
use crate::views::objects::Link;
use crate::widgets::table::model::ObjectId;

/// One list of related objects: its heading, the noun of its objects, its answer and the
/// index that gave it.
struct Group {
    title: &'static str,
    noun: &'static str,
    index: &'static str,
    /// The kind, the identity and the text of the first links, the count of all objects
    /// of the answer and its basis.
    answer: Answer<Links>,
}

/// The first links of a list, and the count of all its objects. A DaemonSet can have
/// thousands of pods, and the panel builds its links in each frame.
struct Links {
    first: Vec<(Kind, ObjectKey, String)>,
    total: usize,
}

impl Links {
    fn of<T>(answer: Answer<Vec<T>>, link: impl Fn(T) -> (Kind, ObjectKey, String)) -> Answer<Self> {
        let total = answer.found.len();
        let first = answer.found.into_iter().take(ITEM_LIMIT).map(link).collect();
        Answer { found: Self { first, total }, basis: answer.basis }
    }
}

/// Draws a heading and the links of each list of the kind. Gives the object of a link that
/// the user clicked.
pub(super) fn show<K: Describe>(
    ui: &mut Ui,
    palette: &Palette,
    data: &ClusterData,
    id: &ObjectId,
    object: &K,
) -> Option<Link> {
    let mut clicked = None;
    for group in groups(data, id, object) {
        heading(ui, palette, group.title);
        if let Some(text) = basis_note(group.noun, group.index, &group.answer.basis) {
            note(ui, palette.warn, &text);
        }
        let found = &group.answer.found;
        if found.total == 0 && group.answer.basis.is_loaded() {
            ui.label(RichText::new("None.").color(palette.text_faint));
        }
        ui.horizontal_wrapped(|ui| {
            links_only(ui);
            for (kind, key, text) in &found.first {
                if ui.link(text.as_str()).clicked() {
                    clicked = Some(Link { cluster: id.cluster.clone(), kind: *kind, key: key.clone() });
                }
            }
        });
        more(ui, palette, found.total);
        ui.add_space(10.0);
    }
    clicked
}

/// Makes the links of a list links and not texts to select: the selection of a text of
/// several words gives the link the role of a text in the accessibility tree.
pub(crate) fn links_only(ui: &mut Ui) {
    ui.style_mut().interaction.selectable_labels = false;
}

/// The lists of the kind of the object.
fn groups<K: Describe>(data: &ClusterData, id: &ObjectId, object: &K) -> Vec<Group> {
    let (owners, references) = (&data.owners, &data.references);
    let named = |kind: Kind, key: ObjectKey| {
        let text = if kind == Kind::Pod { key.name.to_string() } else { format!("{} {}", kind.info().name, key.name) };
        (kind, key, short(&text, 200).into_owned())
    };
    let refs = |answer: Answer<Vec<ObjectRef>>| Links::of(answer, |object| named(object.kind, object.key));
    let pods = |answer: Answer<Vec<ObjectKey>>| Links::of(answer, |key| named(Kind::Pod, key));
    let (kind, uid) = (K::ID, &*id.uid);
    match kind {
        // The pods of a workload, with their state, and the Jobs of a CronJob are parts of
        // the kind (`views/workloads`).
        Kind::Deployment => {
            let answer = refs(owners.owned(kind, uid));
            vec![Group { title: "REPLICASETS", noun: "ReplicaSets", index: OWNERS, answer }]
        }
        // The endpoints of a Service, with their pods, are a part of the kind (`views/network`).
        Kind::Service => {
            let slices = Links::of(owners.slices_of(&id.key), |key| named(Kind::EndpointSlice, key));
            vec![Group { title: "ENDPOINTSLICES", noun: "EndpointSlices", index: OWNERS, answer: slices }]
        }
        // The pods on a Node, with their state, are a part of the kind (`views/cluster`).
        Kind::ConfigMap | Kind::Secret | Kind::ServiceAccount => {
            let answer = pods(references.used_by(kind, &id.key));
            vec![Group { title: "USED BY", noun: "pods", index: "reference index", answer }]
        }
        Kind::Pod => {
            let Some(pod) = (object as &dyn Any).downcast_ref::<Pod>() else {
                return Vec::new();
            };
            // The references come from the spec of the pod itself, so the list is current.
            let uses = Answer { found: references_of(pod).into_iter().collect(), basis: Basis::Current };
            let answer = Links::of(uses, |target: Target| named(target.kind, target.key));
            vec![Group { title: "USES", noun: "objects", index: "pod", answer }]
        }
        _ => Vec::new(),
    }
}

/// The name of the owner index in a note.
const OWNERS: &str = "owner index";

/// What a list of an index is good for, when it is not the state of now. `what` names the
/// objects of the list, for example `pods` or `Events`.
pub(crate) fn basis_note(what: &str, index: &str, basis: &Basis) -> Option<String> {
    match basis {
        Basis::Current => None,
        Basis::Old(source) => Some(format!("The {what} can be old: their store {}.", source.state)),
        Basis::NotLoaded(source) => Some(format!("The {what} are not loaded: their store {}.", source.state)),
        Basis::NotFollowed(IndexState::Failed { reason }) => {
            Some(format!("The {what} can be old: the {index} failed ({reason}). It starts again."))
        }
        Basis::NotFollowed(IndexState::Starting) => Some(format!("The {what} are not loaded: the {index} starts.")),
        Basis::NotFollowed(IndexState::Following | IndexState::Stopped) => {
            Some(format!("The {what} can be old: the {index} stopped."))
        }
    }
}
