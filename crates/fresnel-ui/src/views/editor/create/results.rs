//! The documents of the last run of a tab with the state of each job: the Operations tab
//! has the same jobs with their steps. An apply that a conflict refused lists each field
//! with its manager, and "Apply with force…" asks before it takes them.

use egui::{Label, RichText, ScrollArea, Ui};
use fresnel_core::app::{CoreView, JobView};
use fresnel_core::ops::JobState;

use super::plan::{Item, JobOf, Run, Way};
use crate::format::short;
use crate::theme;
use crate::views::operations::state;
use crate::widgets::tone::Tone;
use crate::widgets::{self, dot};

/// The most rows of the list that show at once; it scrolls for more.
const ROWS: f32 = 5.0;

/// Draws the documents of a run. Gives the items whose apply a conflict refused, each with
/// its fields, when the user asks for a force.
pub(super) fn show(ui: &mut Ui, run: &Run, view: &CoreView) -> Option<Vec<(Item, Vec<String>)>> {
    let palette = theme::palette(ui);
    let jobs = run.jobs(view);
    let conflicts: Vec<(Item, Vec<String>)> = run
        .items
        .iter()
        .zip(&jobs)
        .filter_map(|(item, job)| match job {
            JobOf::Shown(job) => Some((item.clone(), fields(job))),
            JobOf::None | JobOf::Left => None,
        })
        .filter(|(_, fields)| !fields.is_empty())
        .collect();
    let mut force = false;
    ui.horizontal(|ui| {
        let ended = jobs.iter().filter(|job| is_final(job)).count();
        let going = run.items.iter().filter(|item| item.order.is_ok()).count();
        let words = format!("{}: {ended} of {going} jobs ended. The Operations tab has their steps.", run.way.verb());
        ui.label(RichText::new(words).strong());
        if run.way == Way::Apply && !conflicts.is_empty() {
            let button = ui.button("Apply with force…");
            widgets::name(&button, "Apply with force");
            force = button.clicked();
        }
    });
    let height = ROWS * (ui.spacing().interact_size.y + ui.spacing().item_spacing.y);
    let area = ScrollArea::vertical().id_salt("documents of the run").max_height(height).auto_shrink([false, true]);
    area.show(ui, |ui| {
        for (item, job) in run.items.iter().zip(&jobs) {
            let (words, tone) = match (&item.order, job) {
                (Err(reason), _) => (format!("not sent: {reason}"), Tone::Warn),
                (Ok(_), JobOf::None) => ("waits for the core".to_owned(), Tone::Muted),
                (Ok(_), JobOf::Shown(job)) => words(job),
                (Ok(_), JobOf::Left) => ("ended: the journal has its result".to_owned(), Tone::Muted),
            };
            ui.horizontal(|ui| {
                dot(ui, tone.color(palette));
                ui.label(RichText::new(short(&item.label, 120)).strong());
                ui.add(Label::new(RichText::new(short(&words, 400)).color(tone.color(palette))).wrap());
            });
        }
    });
    force.then_some(conflicts)
}

/// The words and the tone of the state of a job, with its error.
fn words(job: &JobView) -> (String, Tone) {
    let (word, tone) = state(&job.report.state);
    let step = job.report.steps.last().map(|step| step.text.as_str());
    match (&job.report.state, step) {
        (JobState::Pending, Some(step)) => (format!("{word}: {step}"), tone),
        (state, _) => match state.error() {
            Some(error) => (format!("{word}: {error}"), tone),
            None => (word.to_owned(), tone),
        },
    }
}

/// The fields of the conflict of a refused apply.
fn fields(job: &JobView) -> Vec<String> {
    match &job.report.state {
        JobState::Failed(error) => error.apply_conflicts().map(str::to_owned).collect(),
        _ => Vec::new(),
    }
}

/// True when each job of the run ended, or the run has no job.
pub(super) fn ended(run: &Run, view: &CoreView) -> bool {
    run.items.iter().zip(&run.jobs(view)).all(|(item, job)| item.order.is_err() || is_final(job))
}

/// True for a job that ended: in the view, or after it left the list of the view.
fn is_final(job: &JobOf) -> bool {
    match job {
        JobOf::Shown(job) => job.report.state.is_final(),
        JobOf::Left => true,
        JobOf::None => false,
    }
}
