//! The revisions of the pod template of a workload, with a rollback to each one (plan
//! section 10, `research/08` section 4.5).
//!
//! The revisions come from a job "revision history" of the core, which reads the workload
//! and its ReplicaSets or ControllerRevisions from the server: a ControllerRevision has no
//! data in its store. The section shows the last such job of the object that sent its read,
//! so the Operations tab and the panel tell the same, and a new read is one click. A newer
//! job that the core did not send, as the second click of a fast double click while the
//! first read runs, is a note: it does not hide the read that runs or its revisions.

use std::time::SystemTime;

use egui::{Grid, RichText, Ui};
use fresnel_core::app::JobView;
use fresnel_core::kinds::Action;
use fresnel_core::ops::{JobState, Operation, Outcome, Revision};

use crate::format::{self, short};
use crate::theme::SMALL;
use crate::views::act::Ask;
use crate::views::detail::fields::{ITEM_LIMIT, NONE, VALUE_LIMIT, heading, more, note};
use crate::views::detail::sections::{Context, Pick};
use crate::views::operations::TEXT_LIMIT;

/// Draws the section. Gives the action of a click: a read, or a rollback to one revision.
pub(super) fn show(ui: &mut Ui, cx: &Context<'_>) -> Option<Pick> {
    let palette = cx.palette;
    heading(ui, palette, "REVISIONS");
    let mut picked = None;
    let read = Pick::Ask(Ask { action: Action::RevisionHistory, revision: None });
    let (last, not_sent) = reads(cx);
    if let Some(job) = not_sent {
        note(ui, palette.warn, &failure(job));
    }
    match last {
        None => {
            note(ui, palette.text_muted, "The revisions come from the server on request.");
            if ui.button("Read the revisions").clicked() {
                picked = Some(read);
            }
        }
        Some(job) => match (&job.report.state, &job.report.outcome) {
            (JobState::Pending, _) => note(ui, palette.info, &format!("Job {} reads the revisions…", job.number)),
            (JobState::Done, Outcome::Revisions(revisions)) => {
                picked = table(ui, cx, revisions);
                if ui.button("Read again").clicked() {
                    picked = Some(read);
                }
            }
            _ => {
                note(ui, palette.warn, &failure(job));
                if ui.button("Read again").clicked() {
                    picked = Some(read);
                }
            }
        },
    }
    ui.add_space(10.0);
    picked
}

/// The last job that sent a read of the revisions of this object, and the newest job after
/// it that the core did not send.
fn reads<'a>(cx: &'a Context<'_>) -> (Option<&'a JobView>, Option<&'a JobView>) {
    let id = cx.id;
    let of_object = cx.view.jobs.jobs.iter().rev().map(|job| &**job).filter(|job| {
        let target = &job.order.target;
        job.order.operation == Operation::History
            && target.cluster == id.cluster
            && target.key == id.key
            && *target.uid == *id.uid
    });
    let mut not_sent = None;
    for job in of_object {
        if !matches!(job.report.state, JobState::NotSent(_)) {
            return (Some(job), not_sent);
        }
        not_sent = not_sent.or(Some(job));
    }
    (None, not_sent)
}

/// "Job 7 did not read the revisions: …" for a job that ended with no revisions.
fn failure(job: &JobView) -> String {
    match job.report.state.error() {
        Some(error) => {
            let error = error.to_string();
            format!("Job {} did not read the revisions: {}", job.number, short(&error, TEXT_LIMIT))
        }
        None => format!("Job {} ended with no revisions.", job.number),
    }
}

/// The revisions, the newest first, with a rollback for each one but the current one.
fn table(ui: &mut Ui, cx: &Context<'_>, revisions: &[Revision]) -> Option<Pick> {
    let palette = cx.palette;
    if revisions.is_empty() {
        note(ui, palette.text_muted, "The server has no revision of this object.");
        return None;
    }
    let mut picked = None;
    Grid::new(("revisions of", &cx.id.cluster, &cx.id.key)).num_columns(5).spacing([12.0, 4.0]).show(ui, |ui| {
        for title in ["Revision", "Created", "Images", "Change cause", ""] {
            ui.label(RichText::new(title).size(11.0).color(palette.text_faint));
        }
        ui.end_row();
        for revision in revisions.iter().rev().take(ITEM_LIMIT) {
            ui.label(revision.number.to_string());
            let created = revision.created.map(|time| format::span(cx.clock.since(SystemTime::from(time))));
            ui.label(RichText::new(created.map_or_else(|| NONE.to_owned(), |age| format!("{age} ago"))).size(SMALL));
            ui.label(short(&revision.images.join(", "), VALUE_LIMIT));
            // The change cause is an annotation: any text of any length.
            let cause = short(revision.change_cause.as_deref().unwrap_or(NONE), VALUE_LIMIT);
            ui.label(RichText::new(cause).color(palette.text_muted));
            if revision.current {
                ui.label(RichText::new("current").color(palette.ok));
            } else if ui.small_button(format!("Roll back to {}…", revision.number)).clicked() {
                picked = Some(Pick::Ask(Ask { action: Action::Rollback, revision: Some(revision.number) }));
            }
            ui.end_row();
        }
    });
    more(ui, palette, revisions.len());
    picked
}
