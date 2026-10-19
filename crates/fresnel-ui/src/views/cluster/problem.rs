//! The state now of the problem of a warning Event, as each view shows it: a short word in
//! the tone of the state, and a hint with the fact of the live object that gives it. The
//! state comes from `fresnel_core::kinds::Witness`; an ongoing problem stands out, an end
//! does not, and the row of an ended one is muted.

use std::time::{Duration, SystemTime};

use egui::{Label, Response, RichText, Ui};
use fresnel_core::kinds::{ProblemNow, ProblemState};

use crate::format::Clock;
use crate::theme::Palette;
use crate::widgets::{self, tone::Tone};

/// The step of the moment of a judgement. A state can change with the time alone (a run of
/// a container proves an end after 10 minutes), so a view judges again at each step, and
/// not at each frame.
const STEP: u64 = 10;

/// The moment of a judgement at `now`: the start of its step, so a moment is never later
/// than now.
pub(crate) fn moment(now: SystemTime) -> SystemTime {
    let seconds = now.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |since| since.as_secs());
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds - seconds % STEP)
}

/// The name of a state, which the text filter of the Events finds too.
pub(crate) fn name(state: ProblemState) -> &'static str {
    match state {
        ProblemState::Ongoing => "ongoing",
        ProblemState::Unknown => "unknown",
        ProblemState::Resolved(_) => "resolved",
        ProblemState::Replaced => "replaced",
        ProblemState::Deleted => "deleted",
    }
}

/// True for a problem that ended, with its object or after it: its row needs no look.
pub(crate) fn ended(state: ProblemState) -> bool {
    match state {
        ProblemState::Ongoing | ProblemState::Unknown => false,
        ProblemState::Resolved(_) | ProblemState::Replaced | ProblemState::Deleted => true,
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
        ProblemState::Resolved(None) | ProblemState::Replaced | ProblemState::Deleted => (name.to_owned(), Tone::Muted),
        ProblemState::Unknown => (name.to_owned(), Tone::Faint),
    }
}

/// The state in a sentence, with the fact of the live object.
pub(crate) fn hint(now: &ProblemNow, clock: &Clock) -> String {
    let why = &now.why;
    match now.state {
        ProblemState::Ongoing => format!("Ongoing now: {why}."),
        ProblemState::Unknown => format!("Unknown: {why}."),
        ProblemState::Resolved(Some(since)) => format!("Resolved at {}: {why}.", clock.date_time(since.into())),
        ProblemState::Resolved(None) => format!("Resolved: {why}."),
        ProblemState::Replaced => format!("Replaced: {why}. The problem can go on in the new one."),
        ProblemState::Deleted => format!("Deleted: {why}."),
    }
}

/// The tone of the reason of a warning and of its mark: only an ongoing problem has the tone
/// of a warning, and an ended one is muted. An unknown one has the normal tone: the app does
/// not tell that it stays, nor that it ended.
pub(crate) fn tone(state: ProblemState) -> Tone {
    match state {
        ProblemState::Ongoing => Tone::Warn,
        ProblemState::Unknown => Tone::Normal,
        ProblemState::Resolved(_) | ProblemState::Replaced | ProblemState::Deleted => Tone::Muted,
    }
}

/// The state as a chip, with the sentence as its hint.
pub(crate) fn chip(ui: &mut Ui, palette: &Palette, clock: &Clock, now: &ProblemNow) -> Response {
    let (text, tone) = word(now, clock);
    widgets::chip(ui, &text, tone.color(palette)).on_hover_text(hint(now, clock))
}

/// The chip and the fact of the live object beside it, which wraps in its own column: a
/// narrow panel has room for them only on a line of their own.
pub(crate) fn with_fact(ui: &mut Ui, palette: &Palette, clock: &Clock, now: &ProblemNow) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        chip(ui, palette, clock, now);
        ui.add(Label::new(RichText::new(&now.why).color(palette.text_muted)).wrap());
    });
}

#[cfg(test)]
mod tests {
    use super::ProblemState::{Deleted, Ongoing, Replaced, Resolved, Unknown};
    use super::*;

    /// The user could not see that a problem ended: only an ongoing one has the tone of a
    /// warning, and an unknown one does not look ended.
    #[test]
    fn only_an_ongoing_problem_has_the_tone_of_a_warning_and_an_ended_one_is_muted() {
        let states = [Ongoing, Unknown, Resolved(None), Replaced, Deleted];
        assert_eq!(states.map(tone), [Tone::Warn, Tone::Normal, Tone::Muted, Tone::Muted, Tone::Muted]);
        assert_eq!(states.map(ended), [false, false, true, true, true]);
    }
}
