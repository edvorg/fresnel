//! The sections of a CronJob: its schedule with the time zone, the concurrency policy, the
//! suspend, the deadline, the history limits, the last schedule and the active Jobs; then its
//! last runs with the state of each Job, its pods and the template of its Jobs.
//!
//! The Jobs come from the store of the full Jobs, which the page of the CronJobs holds for
//! the view (`Command::ShowKinds`), and the owner index gives the Jobs of the CronJob.

use std::cmp::Reverse;

use egui::{Grid, RichText, Ui};
use fresnel_core::kinds::Kind;
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::batch::v1::{CronJob, Job};

use super::job::{duration, state};
use super::{pods, template};
use crate::views::detail::fields::{ITEM_LIMIT, NONE, field, heading, more, note};
use crate::views::detail::lines::{details_heading, line};
use crate::views::detail::related::{basis_note, links_only};
use crate::views::detail::sections::{Context, Pick};
use crate::views::objects::{Link, old_since};
use crate::views::pod_spec::texts::at;
use crate::widgets::dot;

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, cron_job: &CronJob) -> Option<Link> {
    let (spec, status) = (cron_job.spec.as_ref(), cron_job.status.as_ref());
    let clock = cx.clock;
    details_heading(ui, cx);
    line(ui, cx, "Schedule", spec.map(|spec| spec.schedule.clone()));
    line(ui, cx, "Time zone", spec.and_then(|spec| spec.time_zone.clone()).or(Some("of the controller".to_owned())));
    let policy = spec.and_then(|spec| spec.concurrency_policy.clone()).unwrap_or_else(|| "Allow".to_owned());
    line(ui, cx, "Concurrency", Some(policy));
    let suspended = spec.and_then(|spec| spec.suspend) == Some(true);
    line(ui, cx, "Suspend", Some(if suspended { "yes: no new Job starts" } else { "no" }.to_owned()));
    let deadline = spec.and_then(|spec| spec.starting_deadline_seconds);
    line(ui, cx, "Start deadline", deadline.map(|seconds| format!("{seconds} s")));
    let limits = spec.map(|spec| {
        let (successful, failed) =
            (spec.successful_jobs_history_limit.unwrap_or(3), spec.failed_jobs_history_limit.unwrap_or(1));
        format!("{successful} successful, {failed} failed Jobs")
    });
    line(ui, cx, "History limits", limits);
    line(ui, cx, "Last schedule", status.and_then(|status| status.last_schedule_time.as_ref()).map(|t| at(t, clock)));
    line(ui, cx, "Last success", status.and_then(|status| status.last_successful_time.as_ref()).map(|t| at(t, clock)));
    let active = status.and_then(|status| status.active.as_ref()).map(Vec::as_slice).unwrap_or_default();
    let mut clicked = None;
    field(ui, cx.palette, "Active Jobs", |ui| {
        if active.is_empty() {
            ui.label(RichText::new(NONE).color(cx.palette.text_faint));
        }
        ui.horizontal_wrapped(|ui| {
            links_only(ui);
            for reference in active.iter().take(ITEM_LIMIT) {
                let name = reference.name.as_deref().unwrap_or_default();
                if ui.link(name).clicked() {
                    let key = ObjectKey::new(&cx.id.key.namespace, name);
                    clicked = Some(Link { cluster: cx.id.cluster.clone(), kind: Kind::Job, key });
                }
            }
        });
        more(ui, cx.palette, active.len());
    });
    ui.add_space(10.0);
    clicked
}

pub(crate) fn parts(ui: &mut Ui, cx: &Context<'_>, cron_job: &CronJob) -> Option<Pick> {
    let run = runs(ui, cx).map(Pick::from);
    let pod = pods::show(ui, cx, Kind::CronJob).map(Pick::from).or(run);
    let job_spec = cron_job.spec.as_ref().and_then(|spec| spec.job_template.spec.as_ref());
    template(ui, cx, job_spec.map(|spec| &spec.template), false).or(pod)
}

/// The Jobs of the CronJob, the newest first, with the state, the start, the duration and
/// the completions of each one.
fn runs(ui: &mut Ui, cx: &Context<'_>) -> Option<Link> {
    let palette = cx.palette;
    heading(ui, palette, "LAST RUNS");
    let Some(data) = cx.data else {
        note(ui, palette.warn, "The Jobs are not loaded: the cluster is not connected.");
        ui.add_space(10.0);
        return None;
    };
    let answer = data.owners.owned(Kind::CronJob, &cx.id.uid);
    if let Some(text) = basis_note("Jobs", "owner index", &answer.basis) {
        note(ui, palette.warn, &text);
    }
    let Some(jobs) = data.snapshot::<Job>() else {
        note(ui, palette.warn, "The Jobs are not loaded: the store of the full Jobs starts.");
        ui.add_space(10.0);
        return None;
    };
    if let Some(old) = old_since(&jobs, cx.clock) {
        note(ui, palette.warn, &format!("The Jobs are {old}."));
    }
    if answer.found.is_empty() && answer.basis.is_loaded() {
        ui.label(RichText::new("None.").color(palette.text_faint));
    }
    let mut found: Vec<(&ObjectKey, Option<&Job>)> =
        answer.found.iter().map(|job| (&job.key, jobs.items.get(&job.key).map(|job| &**job))).collect();
    let start = |job: Option<&Job>| job.and_then(|job| job.status.as_ref()?.start_time.as_ref()).map(|time| time.0);
    found.sort_by_key(|(key, job)| (Reverse(start(*job)), *key));
    let mut clicked = None;
    if !found.is_empty() {
        let grid = Grid::new(("runs of", &cx.id.cluster, &cx.id.key)).num_columns(4).spacing([12.0, 4.0]);
        grid.show(ui, |ui| {
            links_only(ui);
            for title in ["Job", "State", "Started", "Duration"] {
                ui.label(RichText::new(title).size(11.0).color(palette.text_faint));
            }
            ui.end_row();
            for (key, job) in found.iter().take(ITEM_LIMIT) {
                if ui.link(key.name.as_ref()).clicked() {
                    clicked = Some(Link { cluster: cx.id.cluster.clone(), kind: Kind::Job, key: (*key).clone() });
                }
                let Some(job) = job else {
                    ui.label(RichText::new("not in the store yet").color(palette.text_faint));
                    ui.end_row();
                    continue;
                };
                let (text, tone) = state(job);
                ui.horizontal(|ui| {
                    dot(ui, tone.color(palette));
                    ui.label(RichText::new(text).color(tone.color(palette)));
                });
                let status = job.status.as_ref();
                let started = status.and_then(|status| status.start_time.as_ref());
                ui.label(started.map_or_else(|| NONE.to_owned(), |time| at(time, cx.clock)));
                let completed = status.and_then(|status| status.completion_time.as_ref());
                ui.label(duration(started, completed, cx.clock).unwrap_or_else(|| NONE.to_owned()));
                ui.end_row();
            }
        });
    }
    more(ui, palette, found.len());
    ui.add_space(10.0);
    clicked
}
