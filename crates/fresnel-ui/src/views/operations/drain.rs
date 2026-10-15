//! The drain of the chosen job (plan section 10): each pod of the node with its verdict and
//! its state, the PodDisruptionBudget that holds a pod by its name, and the note that the
//! node stays cordoned after the end of the drain. The drain dialog shows the same verdicts.

use egui::{Label, RichText, ScrollArea, Ui};
use fresnel_core::app::JobView;
use fresnel_core::ops::JobState;
use fresnel_core::ops::drain::{Drain, PodState, Verdict};

use super::TEXT_LIMIT;
use crate::format::{self, short};
use crate::theme::{self, ROW_HEIGHT};
use crate::widgets::mini_table::{self, Column, Text};
use crate::widgets::tone::Tone;

const COLUMNS: &[Column] = &[Column::fill("Pod"), Column::fill("Verdict"), Column::fill("State")];
/// The height of the list of the pods before it scrolls.
const LIST_HEIGHT: f32 = 260.0;

pub(super) fn show(ui: &mut Ui, job: &JobView, drain: &Drain) {
    let palette = theme::palette(ui);
    if drain.cordoned && job.report.state.is_final() {
        let text = match job.report.state {
            JobState::Cancelled => {
                "Cancelled: no new eviction goes out. The node stays cordoned; Uncordon opens it again."
            }
            _ => "The node stays cordoned: it takes no new pod. Uncordon opens it again.",
        };
        ui.add(Label::new(RichText::new(text).color(palette.warn)).wrap());
    }
    if drain.pods.is_empty() {
        return;
    }
    let gone = drain.pods.iter().filter(|pod| pod.state == PodState::Gone).count() as u64;
    let held = drain.pods.iter().filter(|pod| matches!(pod.state, PodState::Blocked { .. })).count() as u64;
    let summary = format!("{}: {} gone, {} held", format::count(drain.pods.len() as u64, "pod"), gone, held);
    ui.label(RichText::new(summary).color(palette.text_muted));
    mini_table::header(ui, ("drain of", job.number), COLUMNS);
    let id = ("pods of the drain", job.number);
    ScrollArea::vertical().id_salt(id).max_height(LIST_HEIGHT).auto_shrink([false, true]).show_rows(
        ui,
        ROW_HEIGHT,
        drain.pods.len(),
        |ui, range| {
            for pod in drain.pods.get(range).unwrap_or_default() {
                let name = Text::new(format!("{}/{}", pod.key.namespace, pod.key.name));
                mini_table::row(ui, (id, &*pod.key.name), COLUMNS, &[name, verdict(&pod.verdict), state(&pod.state)]);
            }
        },
    );
}

/// The verdict of a pod as a cell.
pub(crate) fn verdict(verdict: &Verdict) -> Text {
    match verdict {
        Verdict::Evict(None) => Text::new("evict"),
        Verdict::Evict(Some(warning)) => Text::new(format!("evict: {warning}")).tone(Tone::Warn),
        Verdict::Keep(reason) => Text::new(format!("keep: {reason}")).tone(Tone::Muted),
        Verdict::Block(block) => Text::new(format!("blocks: {}", short(&block.text(), TEXT_LIMIT))).tone(Tone::Error),
    }
}

/// The state of a pod as a cell: a budget that holds the pod shows by its name. A text of
/// the server can have 1 MiB, and a cell lays out each character in each frame.
fn state(state: &PodState) -> Text {
    let text = |text: &str| short(text, TEXT_LIMIT).into_owned();
    match state {
        PodState::Waiting => Text::new("waiting").state(Tone::Faint),
        PodState::Evicting => Text::new("evicting").state(Tone::Info),
        PodState::Blocked { budget: Some(budget), reason, attempts } => {
            let reason = text(reason);
            Text::new(format!("held by the PodDisruptionBudget {budget} ({attempts}×): {reason}")).state(Tone::Warn)
        }
        PodState::Blocked { budget: None, reason, attempts } => {
            Text::new(format!("refused for now ({attempts}×): {}", text(reason))).state(Tone::Warn)
        }
        PodState::Terminating => Text::new("terminating").state(Tone::Info),
        PodState::Gone => Text::new("gone").state(Tone::Ok),
        PodState::Kept => Text::new("kept on the node").state(Tone::Muted),
        PodState::Failed(error) => Text::new(format!("failed: {}", text(&error.to_string()))).state(Tone::Error),
        PodState::Unknown(error) => {
            Text::new(format!("result not known: {}", text(&error.to_string()))).state(Tone::Error)
        }
        PodState::Stopped => Text::new("stopped before its end").state(Tone::Muted),
    }
}
