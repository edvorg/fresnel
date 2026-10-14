//! The pods of a workload with their state (`research/02` section 4.2: a table of pods):
//! the name as a link, Ready, the status of `kubectl get pods`, the restarts and the node.
//!
//! The owner index gives the pods, and its answer tells what it is good for: "no pods" and
//! "the pods are not loaded" never look the same.

use egui::{Grid, RichText, Ui};
use fresnel_core::kinds::{Kind, PodStatus};

use crate::views::detail::fields::{ITEM_LIMIT, NONE, heading, more, note};
use crate::views::detail::related::{basis_note, links_only};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;
use crate::widgets::dot;
use crate::widgets::tone::Tone;

/// Draws the section for a workload of `kind`. Gives the pod that the user clicked.
pub(super) fn show(ui: &mut Ui, cx: &Context<'_>, kind: Kind) -> Option<Link> {
    let palette = cx.palette;
    heading(ui, palette, "PODS");
    let Some(data) = cx.data else {
        note(ui, palette.warn, "The pods are not loaded: the cluster is not connected.");
        ui.add_space(10.0);
        return None;
    };
    let answer = data.owners.pods_of(kind, &cx.id.uid);
    if let Some(text) = basis_note("pods", "owner index", &answer.basis) {
        note(ui, palette.warn, &text);
    }
    if answer.found.is_empty() && answer.basis.is_loaded() {
        ui.label(RichText::new("None.").color(palette.text_faint));
    }
    let mut clicked = None;
    if !answer.found.is_empty() {
        let grid = Grid::new(("pods of", &cx.id.cluster, &cx.id.key)).num_columns(5).spacing([12.0, 4.0]);
        grid.show(ui, |ui| {
            links_only(ui);
            for title in ["Name", "Ready", "Status", "Restarts", "Node"] {
                ui.label(RichText::new(title).size(11.0).color(palette.text_faint));
            }
            ui.end_row();
            for pod in answer.found.iter().take(ITEM_LIMIT) {
                if ui.link(pod.key.name.as_ref()).clicked() {
                    clicked = Some(Link { cluster: cx.id.cluster.clone(), kind: Kind::Pod, key: pod.key.clone() });
                }
                match data.pods.items.get(&pod.key) {
                    Some(object) => {
                        let status = PodStatus::of(object);
                        ui.label(format!("{}/{}", status.ready, status.containers));
                        let tone = Tone::of_object(status.health);
                        ui.horizontal(|ui| {
                            dot(ui, tone.color(palette));
                            ui.label(RichText::new(&status.text).color(tone.color(palette)));
                        });
                        ui.label(status.restarts.to_string());
                        let node = object.spec.as_ref().and_then(|spec| spec.node_name.as_deref());
                        ui.label(RichText::new(node.unwrap_or(NONE)).color(palette.text_muted));
                    }
                    // The index can know a pod before the snapshot of this frame has it.
                    None => {
                        for _ in 0..4 {
                            ui.label(RichText::new(NONE).color(palette.text_faint));
                        }
                    }
                }
                ui.end_row();
            }
        });
    }
    more(ui, palette, answer.found.len());
    ui.add_space(10.0);
    clicked
}
