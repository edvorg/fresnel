//! The credential of one cluster in its card.
//!
//! The text that a plugin wrote shows here, for the user. It is in no log and in no
//! journal entry.

use egui::{RichText, Ui};
use fresnel_core::net::credential::{CredentialFailure, CredentialState, CredentialStatus, RunReason};

use super::quoted;
use crate::format::{self, Clock};
use crate::theme::Palette;
use crate::widgets::tone::Tone;

pub(super) fn show(ui: &mut Ui, palette: &Palette, credential: &CredentialStatus, clock: &Clock) {
    let (tone, text) = state_text(&credential.state, clock);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Credential").color(palette.text_faint));
        ui.label(RichText::new(text).color(tone.color(palette)));
        ui.label(RichText::new(counts_text(credential)).color(palette.text_faint));
    });
    if let Some(failure) = failure(&credential.state).filter(|failure| !failure.detail().is_empty()) {
        quoted(ui, palette, failure.detail());
    }
    ui.add_space(2.0);
}

fn state_text(state: &CredentialState, clock: &Clock) -> (Tone, String) {
    match state {
        CredentialState::None => (Tone::Muted, "no token yet".to_owned()),
        CredentialState::Running { reason, since, last_failure } => {
            let why = match reason {
                RunReason::Start => "for the first token",
                RunReason::Expiry => "because the token expires",
                RunReason::Unauthorized => "because the server refused the token",
                RunReason::Retry => "again after a failure",
            };
            let text = format!("the plugin runs {why}, for {}", format::span(clock.since(*since)));
            match last_failure {
                Some(failure) => (Tone::Error, format!("{text}. The failure before: {failure}")),
                None => (Tone::Info, text),
            }
        }
        CredentialState::Valid { until: Some(until) } => {
            let left = format::span(clock.until(*until));
            (Tone::Ok, format!("valid until {} (in {left})", clock.time_of_day(*until)))
        }
        CredentialState::Valid { until: None } => (Tone::Ok, "valid, with no expiry time".to_owned()),
        CredentialState::Failed { failure, failures, next_attempt } => {
            let next = format::span(clock.until(*next_attempt));
            (Tone::Error, format!("{failure}. Failure {failures} in sequence, next attempt in {next}"))
        }
    }
}

fn failure(state: &CredentialState) -> Option<&CredentialFailure> {
    match state {
        CredentialState::Failed { failure, .. } => Some(failure),
        CredentialState::Running { last_failure, .. } => last_failure.as_ref(),
        CredentialState::None | CredentialState::Valid { .. } => None,
    }
}

fn counts_text(credential: &CredentialStatus) -> String {
    let mut text = format::count(credential.plugin_runs, "plugin run");
    if credential.unauthorized > 0 {
        text.push_str(&format!(", {} of HTTP 401", format::count(credential.unauthorized, "answer")));
    }
    text
}
