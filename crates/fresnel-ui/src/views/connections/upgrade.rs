//! The upgrade connections of one cluster in its card (plan section 8.14): the WebSockets of
//! exec, attach and the tunnels of the forwards, with their counts, their bytes and the reason
//! of the last break, and the warm connections that a new socket takes. A socket is no stream
//! of a channel, so the table of the channels does not show it.

use egui::{RichText, Ui};
use fresnel_core::net::{SocketCounts, UpgradeStatus};

use crate::format;
use crate::theme::Palette;
use crate::widgets::tone::Tone;

pub(super) fn show(ui: &mut Ui, palette: &Palette, upgrade: &UpgradeStatus) {
    let sockets = &upgrade.sockets;
    if *upgrade == UpgradeStatus::default() {
        ui.label(RichText::new("No socket of exec, attach or a tunnel since the start.").color(palette.text_faint));
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Sockets of exec, attach and tunnels").color(palette.text_faint));
        let tone = if sockets.write_waits > 0 { Tone::Warn } else { Tone::Normal };
        ui.label(RichText::new(counts_text(sockets)).color(tone.color(palette)));
        let bytes = format!("{} sent, {} received", format::memory(sockets.sent), format::memory(sockets.received));
        ui.label(RichText::new(bytes).color(palette.text_faint));
    });
    if let Some(failure) = &sockets.last_failure {
        // A break of an earlier socket is history: the counts tell the sockets of now.
        ui.label(RichText::new(format!("The last break of a socket: {failure}")).color(palette.text_faint));
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Warm connections").color(palette.text_faint));
        ui.label(warm_text(upgrade));
        if let Some(error) = &upgrade.last_error {
            ui.label(RichText::new(format!("the last connect failed: {error}")).color(palette.warn));
        }
    });
}

/// "2 open; 14 since the start: 11 ended, 1 broke (1 silent); 1 with a write that waits for
/// its peer".
fn counts_text(sockets: &SocketCounts) -> String {
    let mut text = format!(
        "{} open; {} since the start: {} ended, {} broke",
        format::thousands(sockets.open),
        format::thousands(sockets.opened),
        format::thousands(sockets.closed),
        format::thousands(sockets.failed)
    );
    if sockets.silent > 0 {
        text.push_str(&format!(" ({} silent)", format::thousands(sockets.silent)));
    }
    if sockets.write_waits > 0 {
        text.push_str(&format!("; {} with a write that waits for its peer", format::thousands(sockets.write_waits)));
    }
    text
}

/// "2 ready for 3 forwards and terminals; 15 reused, 4 new, 1 dropped".
fn warm_text(upgrade: &UpgradeStatus) -> String {
    format!(
        "{} ready for {} forwards and terminals; {} reused, {} new, {} dropped",
        format::thousands(upgrade.warm as u64),
        format::thousands(upgrade.holders as u64),
        format::thousands(upgrade.reused),
        format::thousands(upgrade.opened),
        format::thousands(upgrade.dropped)
    )
}
