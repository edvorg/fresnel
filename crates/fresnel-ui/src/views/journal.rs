//! The Journal tab: the events, warnings and errors of the app.
//!
//! The list in memory has a limit. The tab tells how many entries left it; the log file
//! keeps all of them. The tab draws only the rows on the screen.

use std::sync::Arc;

use egui::{Frame, Margin, RichText, ScrollArea, Ui};
use fresnel_core::journal::{Entry, JournalView, Level};

use crate::format::{self, Clock};
use crate::theme::{self, BAR_HEIGHT, PAD, ROW_HEIGHT};
use crate::widgets::mini_table::{self, Column, Text};
use crate::widgets::tone::Tone;

const COLUMNS: &[Column] =
    &[Column::fixed("Time", 64.0), Column::fixed("Level", 76.0), Column::fixed("Source", 150.0), Column::fill("Entry")];

/// The filter of the tab.
#[derive(Debug)]
pub(crate) struct JournalTab {
    /// The tab shows entries of this level and above.
    min_level: Level,
}

impl Default for JournalTab {
    fn default() -> Self {
        Self { min_level: Level::Info }
    }
}

impl JournalTab {
    pub(crate) fn show(&mut self, ui: &mut Ui, journal: &JournalView, clock: &Clock) {
        let palette = theme::palette(ui);
        ui.horizontal(|ui| {
            ui.set_height(BAR_HEIGHT - 4.0);
            ui.add_space(PAD);
            for (level, title) in [(Level::Info, "All"), (Level::Warning, "Warnings"), (Level::Error, "Errors")] {
                ui.selectable_value(&mut self.min_level, level, title);
            }
            let counts =
                format!("{}, {}", format::count(journal.errors, "error"), format::count(journal.warnings, "warning"));
            ui.label(RichText::new(counts).color(palette.text_muted));
            if let Some(text) = dropped_text(journal.dropped()) {
                ui.label(RichText::new(text).color(palette.warn));
            }
        });
        let shown: Vec<&Arc<Entry>> = journal.entries.iter().filter(|entry| entry.level >= self.min_level).collect();
        Frame::new().inner_margin(Margin { left: PAD as i8, ..Margin::ZERO }).show(ui, |ui| {
            // The list computes the place of a row from its height, so rows have no gap.
            ui.spacing_mut().item_spacing.y = 0.0;
            mini_table::header(ui, "journal", COLUMNS);
            let rows = ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true);
            rows.show_rows(ui, ROW_HEIGHT, shown.len(), |ui, range| {
                for entry in shown.get(range).unwrap_or_default() {
                    mini_table::row(ui, entry.number, COLUMNS, &cells(entry, clock));
                }
            });
        });
    }
}

/// The note for entries that left the memory (design rule 12).
fn dropped_text(dropped: u64) -> Option<String> {
    match dropped {
        0 => None,
        1 => Some("1 older entry is only in the log file".to_owned()),
        more => Some(format!("{} older entries are only in the log file", format::thousands(more))),
    }
}

fn cells(entry: &Entry, clock: &Clock) -> Vec<Text> {
    let (tone, level) = Tone::of_level(entry.level);
    vec![
        Text::new(clock.time_of_day(entry.time)).tone(Tone::Faint).mono(),
        Text::new(level).state(tone),
        Text::new(&entry.source).tone(Tone::Muted),
        Text::new(&entry.text),
    ]
}
