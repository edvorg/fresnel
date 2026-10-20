//! The state now of the problem of a warning Event, as each view shows it: a short word in
//! the tone of the state, the fact that gives it on the row itself, and a hint with the
//! whole sentence. The state comes from `fresnel_core::kinds::Witness`; an ongoing problem
//! stands out, an end does not, and the row of an ended one is muted. A quiet one most
//! likely ended: it is muted, and its words say that this is not proved.

use std::time::{Duration, SystemTime};

use egui::{Label, Response, RichText, Ui};
use fresnel_core::kinds::{ProblemNow, ProblemState};
use k8s_openapi::jiff::Timestamp;

use crate::format::{self, Clock};
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
        ProblemState::Quiet(_) => "quiet",
        ProblemState::Resolved(_) => "resolved",
        ProblemState::Replaced => "replaced",
        ProblemState::Deleted => "deleted",
    }
}

/// True for a problem that ended, with its object or after it: its row needs no look. A
/// quiet one did not end with a proof.
pub(crate) fn ended(state: ProblemState) -> bool {
    match state {
        ProblemState::Ongoing | ProblemState::Unknown | ProblemState::Quiet(_) => false,
        ProblemState::Resolved(_) | ProblemState::Replaced | ProblemState::Deleted => true,
    }
}

/// The word of a state with the time of an end, as "resolved 12:31", or the time with no
/// report, as "quiet 35 min", and its tone.
pub(crate) fn word(now: &ProblemNow, clock: &Clock) -> (String, Tone) {
    let name = name(now.state);
    match now.state {
        ProblemState::Ongoing => (name.to_owned(), Tone::Warn),
        ProblemState::Quiet(last) => (format!("{name} {}", silence(last, clock)), Tone::Muted),
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
        ProblemState::Quiet(last) => {
            format!("Quiet for {}, so most likely ended, but not proved: {why}.", silence(last, clock))
        }
        ProblemState::Resolved(Some(since)) => format!("Resolved at {}: {why}.", clock.date_time(since.into())),
        ProblemState::Resolved(None) => format!("Resolved: {why}."),
        ProblemState::Replaced => format!("Replaced: {why}. The problem can go on in the new one."),
        ProblemState::Deleted => format!("Deleted: {why}."),
    }
}

/// The tone of the reason of a warning and of its mark: only an ongoing problem has the tone
/// of a warning, and an ended or a quiet one is muted. An unknown one has the normal tone:
/// the app does not tell that it stays, nor that it ended.
pub(crate) fn tone(state: ProblemState) -> Tone {
    match state {
        ProblemState::Ongoing => Tone::Warn,
        ProblemState::Unknown => Tone::Normal,
        ProblemState::Quiet(_) | ProblemState::Resolved(_) | ProblemState::Replaced | ProblemState::Deleted => {
            Tone::Muted
        }
    }
}

/// The word and the fact in one text, as "unknown: the container web runs since before the
/// last report", for a cell of a table.
pub(crate) fn line(now: &ProblemNow, clock: &Clock) -> (String, Tone) {
    let (word, tone) = word(now, clock);
    (format!("{word}: {}", now.why), tone)
}

/// The time since the last report of a quiet problem, in minutes: it is quiet from
/// `QUIET_AFTER` on, so a second tells nothing.
fn silence(last: Timestamp, clock: &Clock) -> String {
    let silent = clock.since(SystemTime::from(last));
    if silent < Duration::from_secs(3600) { format!("{} min", silent.as_secs() / 60) } else { format::span(silent) }
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
    use super::ProblemState::{Deleted, Ongoing, Quiet, Replaced, Resolved, Unknown};
    use super::*;

    /// The user could not see that a problem ended: only an ongoing one has the tone of a
    /// warning, and an unknown one does not look ended. A quiet one is muted, and not ended.
    #[test]
    fn only_an_ongoing_problem_has_the_tone_of_a_warning_and_an_ended_one_is_muted() {
        let quiet = Quiet(Timestamp::UNIX_EPOCH);
        let states = [Ongoing, Unknown, quiet, Resolved(None), Replaced, Deleted];
        let tones = [Tone::Warn, Tone::Normal, Tone::Muted, Tone::Muted, Tone::Muted, Tone::Muted];
        assert_eq!(states.map(tone), tones);
        assert_eq!(states.map(ended), [false, false, false, true, true, true]);
    }
}
