//! The window of one long line: the pane lays out its start only, and this window has each
//! byte that the view keeps of it, with a copy.

use egui::{Id, Label, RichText, ScrollArea, Window};
use fresnel_core::logs::LogView;

use crate::format;
use crate::theme;

/// The size of the window at its first open.
const SIZE: [f32; 2] = [760.0, 420.0];

/// Draws the window of the line `open` of the view, if one is open. Close sets `open` to `None`.
pub(super) fn show(ctx: &egui::Context, tab: u64, open: &mut Option<u64>, view: &LogView) {
    let Some(number) = *open else {
        return;
    };
    let mut still = true;
    let line = view.lines.get(number);
    let title = line.map_or_else(
        || "The full line".to_owned(),
        |line| format!("The full line: {} bytes", format::thousands(line.text.len() as u64)),
    );
    Window::new(title).id(Id::new(("log full line", tab))).open(&mut still).default_size(SIZE).show(ctx, |ui| {
        let palette = theme::palette(ui);
        let Some(line) = line else {
            ui.label(RichText::new("The line left the view at its limit.").color(palette.warn));
            return;
        };
        ui.horizontal(|ui| {
            if ui.button("Copy the line").clicked() {
                ui.ctx().copy_text(line.text.to_string());
            }
            if line.cut {
                let text = "The line was longer: the app keeps the start of a line up to its limit.";
                ui.label(RichText::new(text).color(palette.warn));
            }
        });
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.add(Label::new(RichText::new(&*line.text).font(theme::mono())).wrap());
        });
    });
    if !still {
        *open = None;
    }
}
