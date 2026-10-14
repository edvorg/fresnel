//! The Operations tab (plan section 7.3): each job of the core with its state, its object
//! and its operation, the newest first, and the chosen job with its steps, its error text,
//! what it gave and a cancel while it runs.
//!
//! A long task such as a rollout status is a job with steps too, so this one tab shows it.
//! The list draws only the rows on the screen: the core keeps 500 jobs.
//!
//! - `job`: the chosen job.

mod job;

pub(crate) use job::TEXT_LIMIT;

use std::sync::Arc;

use egui::{Frame, Margin, Panel, RichText, ScrollArea, Ui};
use fresnel_core::app::{JobView, JobsView};
use fresnel_core::ops::JobState;

use crate::format;
use crate::shell::Scene;
use crate::theme::{self, BAR_HEIGHT, PAD, ROW_HEIGHT};
use crate::widgets::mini_table::{self, Column, Text};
use crate::widgets::tone::Tone;

const COLUMNS: &[Column] = &[
    Column::fixed("Job", 44.0).numeric(),
    Column::fixed("State", 130.0),
    Column::fixed("Cluster", 120.0),
    Column::fill("Operation"),
    Column::fill("Object"),
];

/// The width of the part with the chosen job.
const DETAIL_WIDTH: f32 = 440.0;

/// The job that the user chose. `None`: the newest job.
#[derive(Debug, Default)]
pub(crate) struct OperationsTab {
    chosen: Option<u64>,
}

impl OperationsTab {
    pub(crate) fn show(&mut self, ui: &mut Ui, scene: &mut Scene<'_>) {
        let palette = theme::palette(ui);
        let jobs = Arc::clone(&scene.view.jobs);
        ui.horizontal(|ui| {
            ui.set_height(BAR_HEIGHT - 4.0);
            ui.add_space(PAD);
            ui.label(RichText::new(summary(&jobs)).color(palette.text_muted));
            if jobs.dropped > 0 {
                let text = format!("{} older jobs that ended left the list", format::thousands(jobs.dropped));
                ui.label(RichText::new(text).color(palette.warn));
            }
        });
        if jobs.jobs.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(PAD);
                let text = "No job yet. An action on an object starts one, and this tab shows its steps.";
                ui.label(RichText::new(text).color(palette.text_faint));
            });
            return;
        }
        let newest = jobs.jobs.last().map(|job| job.number);
        let chosen = self.chosen.filter(|number| jobs.jobs.iter().any(|job| job.number == *number)).or(newest);
        let detail = Panel::right("chosen job").resizable(true).default_size(DETAIL_WIDTH).frame(Frame::new());
        detail.show(ui, |ui| {
            if let Some(job) = jobs.jobs.iter().find(|job| Some(job.number) == chosen) {
                Frame::new().inner_margin(Margin::symmetric(PAD as i8, 4)).show(ui, |ui| job::show(ui, scene, job));
            }
        });
        Frame::new().inner_margin(Margin { left: PAD as i8, ..Margin::ZERO }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.style_mut().interaction.selectable_labels = false;
            mini_table::header(ui, "operations", COLUMNS);
            // The newest job first.
            let rows: Vec<&Arc<JobView>> = jobs.jobs.iter().rev().collect();
            ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                for job in rows.get(range).unwrap_or_default() {
                    let name = format!("Job {}", job.number);
                    let row = mini_table::row_to_choose(
                        ui,
                        job.number,
                        COLUMNS,
                        &cells(job),
                        (Some(job.number) == chosen, &name),
                    );
                    if row.clicked() {
                        self.chosen = Some(job.number);
                    }
                }
            });
        });
    }
}

/// "2 running, 14 ended".
fn summary(jobs: &JobsView) -> String {
    let running = jobs.jobs.iter().filter(|job| !job.report.state.is_final()).count() as u64;
    let ended = jobs.jobs.len() as u64 - running;
    format!("{} running, {} ended", format::thousands(running), format::thousands(ended))
}

/// The words and the tone of the state of a job.
pub(crate) fn state(state: &JobState) -> (&'static str, Tone) {
    match state {
        JobState::Pending => ("running", Tone::Info),
        JobState::Done => ("done", Tone::Ok),
        JobState::Failed(_) => ("failed", Tone::Error),
        JobState::NotSent(_) => ("not sent", Tone::Warn),
        JobState::Unknown(_) => ("result not known", Tone::Error),
        JobState::Cancelled => ("cancelled", Tone::Muted),
    }
}

fn cells(job: &JobView) -> Vec<Text> {
    let target = &job.order.target;
    let (words, tone) = state(&job.report.state);
    let key = &target.key;
    let object =
        if key.namespace.is_empty() { key.name.to_string() } else { format!("{}/{}", key.namespace, key.name) };
    vec![
        Text::new(job.number.to_string()).tone(Tone::Muted),
        Text::new(words).state(tone),
        Text::new(target.cluster.as_str()).tone(Tone::Muted),
        Text::new(job.order.operation.label()),
        Text::new(format!("{} {object}", target.kind.info().name)),
    ]
}
