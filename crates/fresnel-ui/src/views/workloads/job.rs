//! The sections of a Job: its state, the counts of its pods, the completions, the limits,
//! the times with the duration and the time of its delete by its time to live (`research/08`
//! section 5.4); then its pods and its template.

use std::time::{Duration, SystemTime};

use egui::Ui;
use fresnel_core::kinds::Kind;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

use super::{pods, template};
use crate::format::{self, Clock};
use crate::views::detail::lines::{details_heading, line, seconds, selector};
use crate::views::detail::sections::{Context, Pick};
use crate::views::objects::Link;
use crate::views::pod_spec::texts::at;
use crate::widgets::tone::Tone;

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, job: &Job) -> Option<Link> {
    let (spec, status) = (job.spec.as_ref(), job.status.as_ref());
    let clock = cx.clock;
    details_heading(ui, cx);
    line(ui, cx, "State", Some(state(job).0));
    let count = |value: Option<i32>| value.unwrap_or(0);
    let pods = status.map(|status| {
        let ready = status.ready.map(|ready| format!(", {ready} ready")).unwrap_or_default();
        format!(
            "{} active{ready}, {} succeeded, {} failed",
            count(status.active),
            count(status.succeeded),
            count(status.failed)
        )
    });
    line(ui, cx, "Pods", pods);
    let completions = spec.and_then(|spec| spec.completions);
    let succeeded = count(status.and_then(|status| status.succeeded));
    let wanted = completions.map_or_else(|| "any".to_owned(), |completions| completions.to_string());
    line(ui, cx, "Completions", Some(format!("{succeeded} of {wanted}")));
    line(ui, cx, "Parallelism", Some(spec.and_then(|spec| spec.parallelism).unwrap_or(1).to_string()));
    line(
        ui,
        cx,
        "Completion mode",
        Some(spec.and_then(|spec| spec.completion_mode.clone()).unwrap_or("NonIndexed".to_owned())),
    );
    line(ui, cx, "Backoff limit", Some(spec.and_then(|spec| spec.backoff_limit).unwrap_or(6).to_string()));
    let started = status.and_then(|status| status.start_time.as_ref());
    let completed = status.and_then(|status| status.completion_time.as_ref());
    line(ui, cx, "Started", started.map(|time| at(time, clock)));
    line(ui, cx, "Completed", completed.map(|time| at(time, clock)));
    line(ui, cx, "Duration", duration(started, completed, clock));
    let deadline = spec.and_then(|spec| spec.active_deadline_seconds);
    line(ui, cx, "Active deadline", deadline.map(|seconds| format!("{seconds} s")));
    let ttl = spec.and_then(|spec| spec.ttl_seconds_after_finished);
    line(ui, cx, "Time to live", seconds(ttl, None).map(|text| deleted_at(text, ttl, finished(job), clock)));
    let suspended = spec.and_then(|spec| spec.suspend) == Some(true);
    line(ui, cx, "Suspend", Some(if suspended { "yes" } else { "no" }.to_owned()));
    line(ui, cx, "Selector", Some(selector(spec.and_then(|spec| spec.selector.as_ref()))));
    let indexes = status.and_then(|status| status.completed_indexes.clone()).filter(|indexes| !indexes.is_empty());
    if indexes.is_some() {
        line(ui, cx, "Completed indexes", indexes);
    }
    ui.add_space(10.0);
    None
}

pub(crate) fn parts(ui: &mut Ui, cx: &Context<'_>, job: &Job) -> Option<Pick> {
    let pod = pods::show(ui, cx, Kind::Job).map(Pick::from);
    template(ui, cx, job.spec.as_ref().map(|spec| &spec.template)).map(Pick::from).or(pod)
}

/// The state of a Job from its conditions, with its tone: complete, failed, suspended,
/// running or waiting for its first pod.
pub(super) fn state(job: &Job) -> (String, Tone) {
    let conditions = job.status.as_ref().and_then(|status| status.conditions.as_ref());
    let set = |kind: &str| conditions.into_iter().flatten().find(|c| c.type_ == kind && c.status == "True");
    if set("Complete").is_some() {
        return ("Complete".to_owned(), Tone::Ok);
    }
    if let Some(failed) = set("Failed").or_else(|| set("FailureTarget")) {
        let reason = failed.reason.as_deref().map(|reason| format!(": {reason}")).unwrap_or_default();
        return (format!("Failed{reason}"), Tone::Error);
    }
    if set("Suspended").is_some() {
        return ("Suspended".to_owned(), Tone::Muted);
    }
    match job.status.as_ref().and_then(|status| status.active).unwrap_or(0) {
        0 => ("Waiting for its pods".to_owned(), Tone::Info),
        active => (format!("Running ({active} active)"), Tone::Info),
    }
}

/// The time of the end of a Job: its completion, or the change to the condition `Failed`.
fn finished(job: &Job) -> Option<&Time> {
    let status = job.status.as_ref()?;
    let failed = status.conditions.iter().flatten().find(|c| c.type_ == "Failed" && c.status == "True");
    status.completion_time.as_ref().or_else(|| failed.and_then(|condition| condition.last_transition_time.as_ref()))
}

/// The time from the start to the completion, or to now for a Job that runs.
pub(super) fn duration(started: Option<&Time>, completed: Option<&Time>, clock: &Clock) -> Option<String> {
    let started = SystemTime::from(started?.0);
    let span = match completed {
        Some(completed) => SystemTime::from(completed.0).duration_since(started).unwrap_or(Duration::ZERO),
        None => clock.since(started),
    };
    Some(format::span(span))
}

/// "3600 s, the TTL controller deletes it at 2027-01-15 09:00:00 (in 1 h)".
fn deleted_at(text: String, ttl: Option<i32>, finished: Option<&Time>, clock: &Clock) -> String {
    let (Some(ttl), Some(finished)) = (ttl, finished) else {
        return text;
    };
    let at = SystemTime::from(finished.0) + Duration::from_secs(u64::try_from(ttl).unwrap_or(0));
    format!("{text}, the TTL controller deletes it at {}", clock.date_time(at))
}
