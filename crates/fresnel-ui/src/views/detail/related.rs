//! The objects that relate to the object of the panel (plan sections 9.8 and 9.9), from
//! the indexes of its cluster: the objects below a workload, the pods behind a Service, the
//! pods that use an object, and what a pod uses. Each one is a link to its panel.
//!
//! Each answer of an index tells what it is good for, and the section shows that next to
//! the links: "no pods" and "the pods are not loaded" never look the same.

use std::any::Any;

use egui::{RichText, Ui};
use fresnel_core::app::ClusterData;
use fresnel_core::index::{Answer, Basis, IndexState, references_of};
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
    /// The kind, the identity and the text of each link.
    answer: Answer<Vec<(Kind, ObjectKey, String)>>,
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
        if found.is_empty() && group.answer.basis.is_loaded() {
            ui.label(RichText::new("None.").color(palette.text_faint));
        }
        ui.horizontal_wrapped(|ui| {
            links_only(ui);
            for (kind, key, text) in found.iter().take(ITEM_LIMIT) {
                if ui.link(text.as_str()).clicked() {
                    clicked = Some(Link { cluster: id.cluster.clone(), kind: *kind, key: key.clone() });
                }
            }
        });
        more(ui, palette, found.len());
        ui.add_space(10.0);
    }
    clicked
}

/// Makes the links of a list links and not texts to select: the selection of a text of
/// several words gives the link the role of a text in the accessibility tree.
pub(super) fn links_only(ui: &mut Ui) {
    ui.style_mut().interaction.selectable_labels = false;
}

/// The lists of the kind of the object.
fn groups<K: Describe>(data: &ClusterData, id: &ObjectId, object: &K) -> Vec<Group> {
    let (owners, references) = (&data.owners, &data.references);
    let named = |kind: Kind, key: ObjectKey| {
        let text = if kind == Kind::Pod { key.name.to_string() } else { format!("{} {}", kind.info().name, key.name) };
        (kind, key, short(&text, 200).into_owned())
    };
    let refs = |answer: Answer<Vec<fresnel_core::index::ObjectRef>>| Answer {
        found: answer.found.into_iter().map(|object| named(object.kind, object.key)).collect(),
        basis: answer.basis,
    };
    let pods = |answer: Answer<Vec<ObjectKey>>| Answer {
        found: answer.found.into_iter().map(|key| named(Kind::Pod, key)).collect(),
        basis: answer.basis,
    };
    let (kind, uid) = (K::ID, &*id.uid);
    let pods_below = || Group { title: "PODS", noun: "pods", index: OWNERS, answer: refs(owners.pods_of(kind, uid)) };
    match kind {
        Kind::Deployment | Kind::CronJob => {
            let (title, noun) =
                if kind == Kind::Deployment { ("REPLICASETS", "ReplicaSets") } else { ("JOBS", "Jobs") };
            vec![Group { title, noun, index: OWNERS, answer: refs(owners.owned(kind, uid)) }, pods_below()]
        }
        Kind::ReplicaSet | Kind::StatefulSet | Kind::DaemonSet | Kind::Job | Kind::ReplicationController => {
            vec![pods_below()]
        }
        Kind::Service => {
            let slices = owners.slices_of(&id.key);
            let slices = Answer {
                found: slices.found.into_iter().map(|key| named(Kind::EndpointSlice, key)).collect(),
                basis: slices.basis,
            };
            let behind = owners.pods_of_service(&id.key);
            let ready = |ready: bool| if ready { "" } else { " (not ready)" };
            let found = behind.found.into_iter().map(|pod| {
                let text = format!("{}{}", short(&pod.pod.name, 200), ready(pod.ready));
                (Kind::Pod, pod.pod, text)
            });
            let behind = Answer { found: found.collect(), basis: behind.basis };
            vec![
                Group { title: "ENDPOINTSLICES", noun: "EndpointSlices", index: OWNERS, answer: slices },
                Group { title: "PODS", noun: "pods", index: OWNERS, answer: behind },
            ]
        }
        Kind::ConfigMap | Kind::Secret | Kind::ServiceAccount | Kind::Node => {
            let answer = pods(references.used_by(kind, &id.key));
            vec![Group { title: "USED BY", noun: "pods", index: "reference index", answer }]
        }
        Kind::Pod => {
            let Some(pod) = (object as &dyn Any).downcast_ref::<Pod>() else {
                return Vec::new();
            };
            // The references come from the spec of the pod itself, so the list is current.
            let found = references_of(pod).into_iter().map(|target| named(target.kind, target.key)).collect();
            vec![Group {
                title: "USES",
                noun: "objects",
                index: "pod",
                answer: Answer { found, basis: Basis::Current },
            }]
        }
        _ => Vec::new(),
    }
}

/// The name of the owner index in a note.
const OWNERS: &str = "owner index";

/// What a list of an index is good for, when it is not the state of now. `what` names the
/// objects of the list, for example `pods` or `Events`.
pub(super) fn basis_note(what: &str, index: &str, basis: &Basis) -> Option<String> {
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
