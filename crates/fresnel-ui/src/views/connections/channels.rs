//! The channels of one cluster as a table: state, ping time, streams in use, the age of
//! the last frame, connect attempts and the last error. Above it, the outage of the cluster.

use std::time::Duration;

use egui::{RichText, Ui};
use fresnel_core::net::{ChannelState, ChannelStatus, ClusterStatus, GroupStatus};

use crate::format::{self, Clock};
use crate::theme::Palette;
use crate::widgets::mini_table::{self, Column, Text};
use crate::widgets::tone::Tone;

const COLUMNS: &[Column] = &[
    Column::fixed("Channel", 110.0),
    Column::fixed("State", 250.0),
    Column::fixed("Ping", 60.0).numeric(),
    Column::fixed("Streams", 70.0).numeric(),
    Column::fixed("Last answer", 90.0).numeric(),
    Column::fixed("Attempts", 70.0).numeric(),
    Column::fill("Last error"),
];
/// The text of a cell with no value.
const NONE: &str = "—";

pub(super) fn show(ui: &mut Ui, palette: &Palette, status: &ClusterStatus, clock: &Clock) {
    if let Some(text) = outage_text(status, clock) {
        ui.label(RichText::new(text).color(palette.warn));
    }
    mini_table::header(ui, "channels", COLUMNS);
    let groups = [("watches", &status.watches), ("requests", &status.requests), ("log streams", &status.logs)];
    for (purpose, group) in groups {
        for (index, channel) in group.channels.iter().enumerate() {
            let name = format!("{purpose} {}", index + 1);
            mini_table::row(ui, &name, COLUMNS, &cells(&name, channel, clock));
        }
    }
    for (purpose, group) in groups {
        if let Some(text) = waiting_text(purpose, group) {
            ui.label(RichText::new(text).color(palette.warn));
        }
    }
}

fn cells(name: &str, channel: &ChannelStatus, clock: &Clock) -> Vec<Text> {
    let (tone, state) = state_text(channel, clock);
    let live = channel.state == ChannelState::Live;
    let value = |text: Option<String>| match text {
        Some(text) => Text::new(text),
        None => Text::new(NONE).tone(Tone::Faint),
    };
    let error = match &channel.last_error {
        // The error of an earlier connection of a live channel is history.
        Some(error) => Text::new(error).tone(if live { Tone::Faint } else { Tone::Warn }),
        None => Text::new(NONE).tone(Tone::Faint),
    };
    vec![
        Text::new(name),
        Text::new(state).state(tone),
        value(channel.ping_time.map(format::short_time)),
        Text::new(format!("{} / {}", channel.streams_open, channel.stream_budget)),
        value(channel.last_frame_age.map(format::span)),
        Text::new(format::thousands(channel.connect_attempts)),
        error,
    ]
}

fn state_text(channel: &ChannelStatus, clock: &Clock) -> (Tone, String) {
    match &channel.state {
        ChannelState::Connecting { attempt } => (Tone::Info, format!("connecting, attempt {attempt}")),
        ChannelState::WaitsForToken => (Tone::Info, "connected, waits for a token".to_owned()),
        ChannelState::Verifying => (Tone::Info, "connected, no answer yet".to_owned()),
        ChannelState::Live => (Tone::Ok, "live".to_owned()),
        ChannelState::Backoff { failures, pause } => {
            let next = format::span(rest_of_pause(channel, *pause, clock));
            (Tone::Warn, format!("{}, next attempt in {next}", format::count(u64::from(*failures), "failure")))
        }
        ChannelState::Retired => (Tone::Faint, "closed, it had no stream".to_owned()),
    }
}

/// Since when no connect worked. In a long outage the channels connect in turns, so the
/// time of the next attempt is the end of the shortest pause of a channel.
fn outage_text(status: &ClusterStatus, clock: &Clock) -> Option<String> {
    let outage = status.outage?;
    let since = format!("No connect worked since {}.", clock.time_of_day(outage.since));
    let Some(turn) = outage.turn else {
        return Some(since);
    };
    let groups = [&status.watches, &status.requests, &status.logs];
    let rests = groups.into_iter().flat_map(|group| &group.channels).filter_map(|channel| match channel.state {
        ChannelState::Backoff { pause, .. } => Some(rest_of_pause(channel, pause, clock)),
        _ => None,
    });
    let next = match rests.min() {
        Some(rest) => format!("the next one starts at {}", clock.time_of_day_after(rest)),
        None => "one is in progress".to_owned(),
    };
    Some(format!("{since} The cluster now gets one connect attempt each {}: {next}.", format::span(turn)))
}

/// The time from now to the end of the pause of a channel. The pause comes from the settings
/// of the core, and the sum of its start and an extreme pause can be past the end of the clock.
fn rest_of_pause(channel: &ChannelStatus, pause: Duration, clock: &Clock) -> Duration {
    pause.saturating_sub(clock.since(channel.since))
}

/// Requests wait only when each connection of the purpose is at its stream budget.
fn waiting_text(purpose: &str, group: &GroupStatus) -> Option<String> {
    match group.waiting {
        0 => None,
        1 => Some(format!("1 request waits for a free stream of the {purpose}.")),
        more => Some(format!("{more} requests wait for a free stream of the {purpose}.")),
    }
}
