//! The texts of a forward: its state with the time of the next attempt, its target pod,
//! its addresses, its counts and the age of the data of its target.

use std::time::SystemTime;

use fresnel_core::app::ForwardRow;
use fresnel_core::forward::{ForwardState, ForwardView, TargetData};

use crate::format::{self, Clock};
use crate::views::detail::fields::NONE;
use crate::widgets::tone::Tone;

/// The state in a few words for the list: its name, and the time to the next attempt of a
/// reconnect. The panel of the chosen forward has its reason.
pub(super) fn short_state(view: &ForwardView, clock: &Clock) -> (String, Tone) {
    let text = match &view.state {
        ForwardState::Reconnecting { next_retry, .. } => format!("reconnecting{}", next(*next_retry, clock)),
        other => other.name().to_owned(),
    };
    (text, tone(&view.state))
}

/// The state in words, with the attempt, the time to the next one and the reason.
pub(super) fn state(view: &ForwardView, clock: &Clock) -> (String, Tone) {
    let text = match &view.state {
        ForwardState::Reconnecting { attempt, next_retry, reason } => {
            let attempt = attempt.map(|attempt| format!(", attempt {attempt}")).unwrap_or_default();
            format!("reconnecting{attempt}{}: {reason}", next(*next_retry, clock))
        }
        other => other.to_string(),
    };
    (text, tone(&view.state))
}

fn tone(state: &ForwardState) -> Tone {
    match state {
        ForwardState::Active => Tone::Ok,
        ForwardState::Starting(_) => Tone::Info,
        ForwardState::Degraded(_) | ForwardState::Reconnecting { .. } => Tone::Warn,
        ForwardState::Failed(_) => Tone::Error,
        ForwardState::Stopped => Tone::Faint,
    }
}

/// ", next in 4 s", or ", next now" once the time of the next attempt is past.
fn next(next_retry: Option<SystemTime>, clock: &Clock) -> String {
    match next_retry.map(|at| clock.until(at)) {
        Some(wait) if !wait.is_zero() => format!(", next in {}", format::span(wait)),
        _ => ", next now".to_owned(),
    }
}

/// The pod of the next tunnels, with a mark for a pod that runs and is not ready.
pub(super) fn pod(view: &ForwardView) -> (String, Tone) {
    match &view.target {
        Some(target) if target.ready => (format!("{}:{}", target.pod.name, target.port), Tone::Normal),
        Some(target) => (format!("{}:{}, not ready", target.pod.name, target.port), Tone::Warn),
        None => (NONE.to_owned(), Tone::Faint),
    }
}

/// The local addresses: "127.0.0.1:8080, [::1]:8080".
pub(super) fn addresses(view: &ForwardView) -> String {
    match view.addresses.as_slice() {
        [] => NONE.to_owned(),
        addresses => addresses.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "),
    }
}

/// The first local address, for the list: the panel of the chosen forward has each one.
pub(super) fn first_address(view: &ForwardView) -> String {
    view.addresses.first().map_or_else(|| NONE.to_owned(), ToString::to_string)
}

/// "2 open, 14 in all, 1 reset".
pub(super) fn connections(view: &ForwardView) -> String {
    let counts = view.connections;
    let reset = if counts.reset > 0 { format!(", {} reset", format::thousands(counts.reset)) } else { String::new() };
    format!("{} open, {} in all{reset}", format::thousands(counts.open), format::thousands(counts.total))
}

/// "12Mi sent, 300Ki received": the bytes to the pods and from them.
pub(super) fn bytes(view: &ForwardView) -> String {
    format!("{} sent, {} received", format::memory(view.bytes.sent), format::memory(view.bytes.received))
}

/// How fresh the data is that chose the pod.
pub(super) fn data(view: &ForwardView, clock: &Clock) -> (String, Tone) {
    match &view.data {
        TargetData::Current => ("current".to_owned(), Tone::Normal),
        TargetData::NotLoaded => ("not loaded".to_owned(), Tone::Warn),
        TargetData::Old { kind, since } => {
            let age = format::span(clock.since(*since));
            (format!("old for {age}: the {} are not live", kind.info().title), Tone::Warn)
        }
    }
}

/// The last failure with its age: "12 s ago: a tunnel broke".
pub(super) fn last_error(view: &ForwardView, clock: &Clock) -> (String, Tone) {
    match &view.last_error {
        Some((time, text)) => (format!("{} ago: {text}", format::span(clock.since(*time))), Tone::Warn),
        None => (NONE.to_owned(), Tone::Faint),
    }
}

/// "3 forwards: 2 active, 1 reconnecting", the states in the sequence of their names.
pub(crate) fn summary(rows: &[ForwardRow]) -> String {
    let mut counts: Vec<(&str, u64)> = Vec::new();
    for row in rows {
        let name = row.view.state.name();
        match counts.iter_mut().find(|(state, _)| *state == name) {
            Some((_, count)) => *count += 1,
            None => counts.push((name, 1)),
        }
    }
    counts.sort_unstable();
    let states: Vec<String> = counts.iter().map(|(state, count)| format!("{count} {state}")).collect();
    format!("{}: {}", format::count(rows.len() as u64, "forward"), states.join(", "))
}

#[cfg(test)]
mod tests;
