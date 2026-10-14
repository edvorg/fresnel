//! The state lines of the key view: its heading, how far the read of its object is, and the
//! state of its last save in the words of the job states of the operations (plan section
//! 7.3): pending, done, failed, not sent, result not known, and the conflict of HTTP 409.

use egui::{Label, RichText, Ui};
use fresnel_core::app::{KeysView, ReadState, SaveJob};
use fresnel_core::cluster::ClusterId;
use fresnel_core::keys::{Draft, WriteState};

use crate::format;
use crate::theme::Palette;
use crate::widgets::tone::Tone;

/// The tone and the word of the state of a write.
pub(super) fn word(state: &WriteState) -> (Tone, &'static str) {
    match state {
        WriteState::Pending => (Tone::Info, "pending"),
        WriteState::Done(_) => (Tone::Ok, "done"),
        WriteState::Conflict(_) => (Tone::Warn, "conflict"),
        WriteState::Failed(_) => (Tone::Error, "failed"),
        WriteState::NotSent(_) => (Tone::Error, "not sent"),
        WriteState::Unknown(_) => (Tone::Error, "result not known"),
    }
}

fn line(ui: &mut Ui, palette: &Palette, tone: Tone, text: &str) {
    ui.add(Label::new(RichText::new(text).color(tone.color(palette))).wrap());
}

/// How far the read is. `view` is the view of the core for the object, `None` before the
/// core has one or while the cluster is not connected. `version` is the version of the
/// values that the panel shows: it keeps them through a failed read and a disconnect.
pub(super) fn read(
    ui: &mut Ui,
    palette: &Palette,
    cluster: &ClusterId,
    connected: bool,
    view: Option<&KeysView>,
    version: Option<&str>,
) {
    let version = version.map(|version| format!("version {version}"));
    let (tone, text) = match (connected, view.map(|view| &view.read), version) {
        (false, _, Some(version)) => {
            (Tone::Warn, format!("{cluster} is not connected. The values are of the last read, {version}."))
        }
        (false, _, None) => (Tone::Warn, format!("{cluster} is not connected, so the values are not read.")),
        (true, None | Some(ReadState::Reading | ReadState::Read), None) => {
            (Tone::Muted, format!("The values are read from {cluster}…"))
        }
        (true, Some(ReadState::Reading), Some(version)) => (Tone::Faint, format!("{version}, read again…")),
        (true, Some(ReadState::Failed(error)), None) => {
            (Tone::Error, format!("The read of the values failed: {error}"))
        }
        (true, Some(ReadState::Failed(error)), Some(version)) => {
            (Tone::Warn, format!("The read failed: {error}. The values are of {version}."))
        }
        (true, Some(ReadState::Read) | None, Some(version)) => (Tone::Faint, format!("{version} of {cluster}")),
    };
    line(ui, palette, tone, &text);
}

/// The state of the last save. `kind` names the kind of the object, as `Secret`.
pub(super) fn save(ui: &mut Ui, palette: &Palette, kind: &str, job: &SaveJob) {
    let keys = format::count(job.save.writes.len() as u64, "key");
    let (tone, word) = word(&job.state);
    let text = match &job.state {
        WriteState::Pending => format!("Save of {keys}: {word}…"),
        WriteState::Done(object) => format!("Save of {keys}: {word}. The {kind} has version {}.", object.version),
        WriteState::Conflict(_) => format!(
            "Save of {keys}: {word}. The {kind} changed on the server after the read, and nothing changed. \
             The panel reads it again, and each key that you and the server changed shows below."
        ),
        WriteState::Failed(error) => {
            format!("Save of {keys}: {word}. The server refused it, and nothing changed: {error}")
        }
        WriteState::NotSent(error) => format!("Save of {keys}: {word}: {error}"),
        WriteState::Unknown(error) => format!(
            "Save of {keys}: {word}: {error}. The app does not send it again; the panel reads the {kind} to show it."
        ),
    };
    line(ui, palette, tone, &text);
}

/// The heading of the section, with the type of a Secret and the count of its keys.
pub(super) fn heading(ui: &mut Ui, palette: &Palette, draft: Option<&Draft>) -> egui::Response {
    let response = ui.label(RichText::new("DATA").size(11.0).color(palette.text_faint));
    if let Some(draft) = draft {
        let base = draft.base();
        let mut parts = Vec::new();
        if let Some(kind) = &base.secret_type {
            parts.push(format!("type {}", kind.name()));
        }
        parts.push(format::count(base.entries.len() as u64, "key"));
        if base.immutable {
            parts.push("immutable".to_owned());
        }
        ui.label(RichText::new(parts.join(" · ")).color(palette.text_muted));
    }
    response
}
