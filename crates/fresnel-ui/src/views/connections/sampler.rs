//! The sampler of the metrics of one cluster in its card (plan section 12.4): the state of
//! each of its two lists with the time of its last answer, the time of the next poll, and
//! the size of the history with the samples that its memory limit dropped (design rule 12).

use egui::{RichText, Ui};
use fresnel_core::metrics::{Metrics, PollState, PollStatus};

use crate::format::{self, Clock};
use crate::theme::Palette;
use crate::widgets::tone::Tone;

pub(super) fn show(ui: &mut Ui, palette: &Palette, metrics: &Metrics, clock: &Clock) {
    for (list, status) in [("PodMetrics", &metrics.pods), ("NodeMetrics", &metrics.nodes)] {
        let (tone, text) = state_text(status, clock);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(list).color(palette.text_faint));
            ui.label(RichText::new(text).color(tone.color(palette)));
            ui.label(RichText::new(counts_text(status)).color(palette.text_faint));
        });
    }
    let next = metrics.next.map(|next| format!(", next in {}", format::span(clock.until(next))));
    let mut text = format!("A poll each {}{}", format::span(metrics.pause()), next.unwrap_or_default());
    text.push_str(&format!(
        ". History: {} series, {}",
        format::thousands(metrics.series_count() as u64),
        format::memory(metrics.bytes() as u64)
    ));
    if metrics.dropped() > 0 {
        text.push_str(&format!(", {} dropped by its memory limit", format::count(metrics.dropped(), "sample")));
    }
    ui.label(RichText::new(format!("{text}.")).color(palette.text_faint));
    ui.add_space(2.0);
}

/// The state of one list with the time of its last answer.
fn state_text(status: &PollStatus, clock: &Clock) -> (Tone, String) {
    let answered = status.answered.map(|time| format!("last answer {} ago", format::span(clock.since(time))));
    let tone = match &status.state {
        PollState::Ok => return (Tone::Ok, format!("ok, {}", answered.unwrap_or_default())),
        PollState::Waiting => return (Tone::Info, status.state.to_string()),
        PollState::Stopped => return (Tone::Muted, status.state.to_string()),
        PollState::NotInstalled(_) | PollState::Forbidden(_) => Tone::Warn,
        PollState::Unavailable(..) | PollState::Timeout(_) | PollState::Error(_) | PollState::InternalError(_) => {
            Tone::Error
        }
    };
    let last = answered.unwrap_or_else(|| "no answer yet".to_owned());
    (tone, format!("{} since {}; {last}", status.state, clock.time_of_day(status.since)))
}

/// The items of the last answer, and the items that the sampler could not read.
fn counts_text(status: &PollStatus) -> String {
    let mut text = format::count(status.items as u64, "item");
    if status.unread > 0 {
        let problem = status.problem.as_deref().unwrap_or_default();
        text.push_str(&format!(", {} unread: {problem}", format::thousands(status.unread)));
    }
    text
}
