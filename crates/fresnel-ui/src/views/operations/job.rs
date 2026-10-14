//! The chosen job of the Operations tab: its operation, its object with the color of its
//! cluster, its state with the error text, each step with its result, what it gave, and
//! a cancel while it runs. A write whose result is not known says that the app did not
//! send it again (design rule 6).

use egui::{Label, RichText, ScrollArea, Ui};
use fresnel_core::app::{Command, JobView};
use fresnel_core::ops::{JobState, Outcome, Rollout, StepState};

use super::state;
use crate::format::short;
use crate::shell::Scene;
use crate::theme;
use crate::views::detail::fields::{ITEM_LIMIT, VALUE_LIMIT};
use crate::widgets::tone::Tone;
use crate::widgets::{self, dot};

/// The most characters of a manifest that the tab shows; the copy has each one.
const MANIFEST_LIMIT: usize = 4000;

pub(super) fn show(ui: &mut Ui, scene: &mut Scene<'_>, job: &JobView) {
    let palette = theme::palette(ui);
    let target = &job.order.target;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Job {}: {}", job.number, job.order.operation.label())).strong());
        if !job.report.state.is_final() {
            let cancel = ui.small_button("Cancel");
            widgets::name(&cancel, &format!("Cancel job {}", job.number));
            let cancel = cancel.on_hover_text("A wait ends at once. A write that is on its way runs to its answer.");
            if cancel.clicked() {
                scene.commands.push(Command::Cancel(job.number));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        dot(ui, scene.colors.color(&target.cluster, palette));
        ui.label(target.cluster.as_str());
        let key = &target.key;
        let namespace = if key.namespace.is_empty() { String::new() } else { format!("{} ", key.namespace) };
        let text = format!("{namespace}{} {}", target.kind.info().name, key.name);
        ui.label(RichText::new(short(&text, VALUE_LIMIT)).color(palette.text_muted));
    });
    let (words, tone) = state(&job.report.state);
    ui.horizontal_wrapped(|ui| {
        dot(ui, tone.color(palette));
        ui.label(RichText::new(words).color(tone.color(palette)));
        if let Some(error) = job.report.state.error() {
            ui.add(Label::new(RichText::new(error.to_string()).color(tone.color(palette))).wrap());
        }
    });
    if let JobState::Unknown(_) = job.report.state {
        let text =
            "The write went out and no answer came back. The app did not send it again; the last step read the object.";
        ui.add(Label::new(RichText::new(text).color(palette.warn)).wrap());
    }
    ui.add_space(4.0);
    ScrollArea::vertical().id_salt(("steps of", job.number)).auto_shrink([false, true]).show(ui, |ui| {
        for step in job.report.steps.iter().take(ITEM_LIMIT) {
            let (mark, tone, result) = match &step.state {
                StepState::Running => ("…", Tone::Info, None),
                StepState::Done(result) => ("✓", Tone::Ok, Some(result.as_str())),
                StepState::Failed => ("✗", Tone::Error, None),
                StepState::Stopped => ("–", Tone::Muted, None),
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(mark).color(tone.color(palette)));
                ui.label(&step.text);
                if let Some(result) = result.filter(|result| !result.is_empty()) {
                    ui.label(RichText::new(format!("— {result}")).color(palette.text_muted));
                }
            });
        }
        outcome(ui, job);
    });
}

/// What the job gave.
fn outcome(ui: &mut Ui, job: &JobView) {
    let palette = theme::palette(ui);
    match &job.report.outcome {
        Outcome::None | Outcome::Written(_) => {}
        Outcome::Revisions(revisions) => {
            let numbers: Vec<String> = revisions.iter().map(|revision| revision.number.to_string()).collect();
            let text = format!("Revisions {}. The panel of the object lists them with a rollback.", numbers.join(", "));
            ui.add(Label::new(text).wrap());
        }
        Outcome::Rollout(rollout) => {
            let (text, tone) = match rollout {
                Rollout::Waiting(text) => (text, Tone::Info),
                Rollout::Complete(text) => (text, Tone::Ok),
                Rollout::Failed(text) => (text, Tone::Error),
            };
            ui.add(Label::new(RichText::new(text).color(tone.color(palette))).wrap());
        }
        Outcome::Manifest(manifest) => {
            let text = "The old Job is gone. Its manifest, for a create by hand:";
            ui.add(Label::new(RichText::new(text).color(palette.warn)).wrap());
            if ui.button("Copy the manifest").clicked() {
                ui.ctx().copy_text(manifest.to_string());
            }
            ui.add(Label::new(RichText::new(short(manifest, MANIFEST_LIMIT)).font(theme::mono())).wrap());
        }
    }
}
