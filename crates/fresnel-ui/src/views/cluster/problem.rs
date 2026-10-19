//! The state now of the problem of a warning Event, as each view shows it: a short word in
//! the tone of the state, and a hint with the fact of the live object that gives it. The
//! state comes from `fresnel_core::kinds::Witness`; an ongoing problem stands out, an end
//! does not.

use std::time::SystemTime;

use egui::{Response, Ui};
use fresnel_core::kinds::{ProblemNow, ProblemState};

use crate::format::Clock;
use crate::theme::Palette;
use crate::widgets::{self, tone::Tone};

/// The name of a state, which the text filter of the Events finds too.
pub(crate) fn name(state: ProblemState) -> &'static str {
    match state {
        ProblemState::Ongoing => "ongoing",
        ProblemState::Resolved(_) => "resolved",
        ProblemState::Unknown => "unknown",
        ProblemState::Gone => "gone",
    }
}

/// The word of a state with the time of an end, as "resolved 12:31", and its tone.
pub(crate) fn word(now: &ProblemNow, clock: &Clock) -> (String, Tone) {
    let name = name(now.state);
    match now.state {
        ProblemState::Ongoing => (name.to_owned(), Tone::Warn),
        ProblemState::Resolved(Some(since)) => {
            (format!("{name} {}", clock.hour_minute(SystemTime::from(since))), Tone::Muted)
        }
        ProblemState::Resolved(None) | ProblemState::Gone => (name.to_owned(), Tone::Muted),
        ProblemState::Unknown => (name.to_owned(), Tone::Faint),
    }
}

/// The state in a sentence, with the fact of the live object.
pub(crate) fn hint(now: &ProblemNow, clock: &Clock) -> String {
    let why = &now.why;
    match now.state {
        ProblemState::Ongoing => format!("Ongoing now: {why}."),
        ProblemState::Resolved(Some(since)) => format!("Resolved at {}: {why}.", clock.date_time(since.into())),
        ProblemState::Resolved(None) => format!("Resolved: {why}."),
        ProblemState::Unknown => format!("Unknown: {why}."),
        ProblemState::Gone => format!("Gone: {why}."),
    }
}

/// The state as a chip, with the sentence as its hint.
pub(crate) fn chip(ui: &mut Ui, palette: &Palette, clock: &Clock, now: &ProblemNow) -> Response {
    let (text, tone) = word(now, clock);
    widgets::chip(ui, &text, tone.color(palette)).on_hover_text(hint(now, clock))
}
