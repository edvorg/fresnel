//! The pods of a workload with their state (`research/02` section 4.2: a table of pods):
//! the name as a link, Ready, the status of `kubectl get pods`, the restarts and the node.
//! The panel of a Node has the same table, with the namespace of each pod in place of
//! the node.
//!
//! The owner index gives the pods, and its answer tells what it is good for: "no pods" and
//! "the pods are not loaded" never look the same.

use egui::{RichText, Ui};
use fresnel_core::app::ClusterData;
use fresnel_core::index::Basis;
use fresnel_core::kinds::{Kind, PodStatus};
use fresnel_core::store::ObjectKey;

use crate::views::detail::fields::{NONE, heading, note};
use crate::views::detail::lines::{none, table};
use crate::views::detail::related::basis_note;
use crate::views::detail::sections::Context;
use crate::views::objects::Link;
use crate::widgets::dot;
use crate::widgets::tone::Tone;

/// The column of a table of pods after the restarts.
#[derive(Clone, Copy)]
pub(crate) enum Last {
    /// The node of each pod: the pods of a workload.
    Node,
    /// The namespace of each pod: the pods on a node.
    Namespace,
}

/// Draws the section for a workload of `kind`. Gives the pod that the user clicked.
pub(super) fn show(ui: &mut Ui, cx: &Context<'_>, kind: Kind) -> Option<Link> {
    heading(ui, cx.palette, "PODS");
    let Some(data) = cx.data else {
        note(ui, cx.palette.warn, "The pods are not loaded: the cluster is not connected.");
        ui.add_space(10.0);
        return None;
    };
    let answer = data.owners.pods_of(kind, &cx.id.uid);
    let keys: Vec<&ObjectKey> = answer.found.iter().map(|pod| &pod.key).collect();
    let clicked = pods(ui, cx, data, (&keys, &answer.basis, "owner index"), Last::Node);
    ui.add_space(10.0);
    clicked
}

/// The pods of an answer of an index, with their state from the snapshot of the pods, and
/// a note when the answer is not current. Gives the pod that the user clicked.
pub(crate) fn pods(
    ui: &mut Ui,
    cx: &Context<'_>,
    data: &ClusterData,
    (keys, basis, index): (&[&ObjectKey], &Basis, &str),
    last: Last,
) -> Option<Link> {
    let palette = cx.palette;
    if let Some(text) = basis_note("pods", index, basis) {
        note(ui, palette.warn, &text);
    }
    if keys.is_empty() && basis.is_loaded() {
        none(ui, cx);
    }
    let mut clicked = None;
    let titles = ["Name", "Ready", "Status", "Restarts", if matches!(last, Last::Node) { "Node" } else { "Namespace" }];
    table(ui, cx, "pods of", &titles, keys, |ui, key| {
        if ui.link(key.name.as_ref()).clicked() {
            clicked = Some(Link { cluster: cx.id.cluster.clone(), kind: Kind::Pod, key: (*key).clone() });
        }
        match data.pods.items.get(key) {
            Some(object) => {
                let status = PodStatus::of(object);
                ui.label(format!("{}/{}", status.ready, status.containers));
                let tone = Tone::of_object(status.health);
                ui.horizontal(|ui| {
                    dot(ui, tone.color(palette));
                    ui.label(RichText::new(&status.text).color(tone.color(palette)));
                });
                ui.label(status.restarts.to_string());
                let text = match last {
                    Last::Node => object.spec.as_ref().and_then(|spec| spec.node_name.as_deref()).unwrap_or(NONE),
                    Last::Namespace => &*key.namespace,
                };
                ui.label(RichText::new(text).color(palette.text_muted));
            }
            // The index can know a pod before the snapshot of this frame has it.
            None => {
                for _ in 0..4 {
                    ui.label(RichText::new(NONE).color(palette.text_faint));
                }
            }
        }
    });
    clicked
}
