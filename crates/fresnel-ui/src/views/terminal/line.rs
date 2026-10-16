//! The line above a terminal (design rule 1): the state of its session in words, its size,
//! the title that its program set, the place of the view in the history, and the input that
//! waits for the session.

use egui::{Button, RichText, Ui};
use fresnel_core::terminal::{GridSize, SessionState, Terminal};

use crate::format::{self, Clock, short};
use crate::theme::{self, SMALL};
use crate::widgets::dot;
use crate::widgets::tone::Tone;

/// The most characters of a title of the remote program on the line.
const TITLE: usize = 80;

/// The tone of a state, also for the title of its tab.
pub(super) fn tone(state: &SessionState) -> Tone {
    match state {
        SessionState::Starting { .. } | SessionState::Connecting | SessionState::Closing => Tone::Info,
        SessionState::Open => Tone::Ok,
        SessionState::Exited { code: 0 } | SessionState::Ended { .. } => Tone::Muted,
        SessionState::InputWaits { .. } | SessionState::Exited { .. } => Tone::Warn,
        SessionState::Failed { .. } => Tone::Error,
    }
}

/// A state in words.
pub(super) fn words(state: &SessionState, clock: &Clock) -> String {
    match state {
        SessionState::Starting { step } => format!("Starting: {step}"),
        SessionState::Connecting => "Connecting".to_owned(),
        SessionState::Open => "Open".to_owned(),
        SessionState::InputWaits { since } => {
            format!("The program reads no input since {}", format::span(clock.since(*since)))
        }
        SessionState::Closing => "Closing: the terminal ends its program in the pod".to_owned(),
        SessionState::Exited { code } => format!("Ended with exit code {code}"),
        SessionState::Ended { reason } => format!("Ended: {reason}"),
        SessionState::Failed { reason } => format!("Failed: {reason}"),
    }
}

/// What the user asked for on the line.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Asked {
    /// Drop the input that waits, such as the rest of a long paste.
    DropInput,
    /// Show the end of the history again.
    ToEnd,
}

/// What the line shows of the widget: the size of the grid, and the lines of the history
/// above the screen and in all.
pub(super) struct Widget {
    pub(super) size: GridSize,
    pub(super) history: (usize, usize),
}

/// Draws the line with the facts of the last frame of the widget.
pub(super) fn show(ui: &mut Ui, terminal: &Terminal, widget: &Widget, clock: &Clock) -> Option<Asked> {
    let palette = theme::palette(ui);
    let state = terminal.state();
    let (tone, words) = (tone(&state), words(&state, clock));
    let mut asked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        dot(ui, tone.color(palette));
        let shown = ui.label(RichText::new(short(&words, 120)).size(SMALL).color(tone.color(palette)));
        if words.chars().count() > 120 {
            shown.on_hover_text(words);
        }
        let faint = |text: String| RichText::new(text).size(SMALL).color(palette.text_faint);
        ui.label(faint(format!("{} × {}", widget.size.columns, widget.size.rows)));
        if let Some(title) = terminal.title().filter(|title| !title.is_empty()) {
            ui.label(faint(format!("“{}”", short(&title, TITLE))));
        }
        let (up, lines) = widget.history;
        if up > 0 {
            let text =
                format!("{} up in a history of {}", format::count(up as u64, "line"), format::thousands(lines as u64));
            ui.label(RichText::new(text).size(SMALL).color(palette.info));
            if ui.add(Button::new(RichText::new("To the end").size(SMALL)).small()).clicked() {
                asked = Some(Asked::ToEnd);
            }
        }
        let waiting = terminal.waiting_input();
        if waiting > 0 {
            let text = format!("{} of input wait for the program", format::count(waiting as u64, "byte"));
            ui.label(RichText::new(text).size(SMALL).color(palette.warn));
            if ui.add(Button::new(RichText::new("Drop the input").size(SMALL)).small()).clicked() {
                asked = Some(Asked::DropInput);
            }
        }
        let lost = terminal.refused_answers();
        if lost > 0 {
            let text = format!("{} of answers did not fit: the program reads no input", format::count(lost, "byte"));
            ui.label(RichText::new(text).size(SMALL).color(palette.warn));
        }
    });
    asked
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    #[test]
    fn each_state_has_its_tone_and_its_words() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let clock = Clock::fixed(now);
        let waits = SessionState::InputWaits { since: now - Duration::from_secs(12) };
        assert_eq!(
            (tone(&waits), words(&waits, &clock).as_str()),
            (Tone::Warn, "The program reads no input since 12 s")
        );
        let starting = SessionState::Starting { step: "the pod kube-system/n1 is created".to_owned() };
        assert_eq!(
            (tone(&starting), words(&starting, &clock)),
            (Tone::Info, "Starting: the pod kube-system/n1 is created".to_owned())
        );
        let (ended, normal) = (SessionState::Exited { code: 130 }, SessionState::Exited { code: 0 });
        assert_eq!((tone(&ended), words(&ended, &clock).as_str()), (Tone::Warn, "Ended with exit code 130"));
        assert_eq!(tone(&normal), Tone::Muted, "a normal end");
        let gone = SessionState::Ended { reason: "the app closed the session".to_owned() };
        assert_eq!((tone(&gone), words(&gone, &clock).as_str()), (Tone::Muted, "Ended: the app closed the session"));
        let failed = SessionState::Failed { reason: "connection reset".to_owned() };
        assert_eq!((tone(&failed), words(&failed, &clock).as_str()), (Tone::Error, "Failed: connection reset"));
        assert_eq!((tone(&SessionState::Open), words(&SessionState::Open, &clock).as_str()), (Tone::Ok, "Open"));
    }
}
