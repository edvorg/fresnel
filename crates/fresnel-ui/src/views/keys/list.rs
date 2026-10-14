//! The keys that the panel lists: the filter of the names, the names that pass it, and the
//! limit of the rows of one frame. A ConfigMap can have thousands of keys, and each row is a
//! layout in each frame, so the panel lists the first ones and the filter finds the others.

use egui::{RichText, TextEdit, Ui};
use fresnel_core::keys::Draft;

use crate::format;
use crate::theme::Palette;
use crate::widgets;

/// The most rows of keys that the panel lays out in one frame.
const KEY_LIMIT: usize = 100;
/// The panel has a filter of the keys from this count of keys on.
const FILTER_FROM: usize = 12;

#[derive(Default)]
pub(super) struct KeyList {
    filter: String,
    /// The names that pass the filter, for the serial of a draft and a filter.
    names: Option<(u64, String, Vec<String>)>,
}

impl KeyList {
    /// Draws the filter when the draft has many keys, and gives the names of the rows of
    /// this frame. `serial` grows with each change of the draft: the names stay until then.
    pub(super) fn names(&mut self, ui: &mut Ui, palette: &Palette, draft: &Draft, serial: u64) -> Vec<String> {
        if draft.base().entries.len() + draft.changes().len() >= FILTER_FROM {
            let field = TextEdit::singleline(&mut self.filter).hint_text("Filter the keys").desired_width(220.0);
            widgets::name(&ui.add(field), "Filter the keys");
        }
        let fresh = self.names.as_ref().is_some_and(|(of, filter, _)| *of == serial && *filter == self.filter);
        if !fresh {
            let needle = self.filter.to_lowercase();
            let keys =
                draft.keys().into_iter().map(|key| key.name).filter(|name| name.to_lowercase().contains(&needle));
            self.names = Some((serial, self.filter.clone(), keys.map(str::to_owned).collect()));
        }
        let names = self.names.as_ref().map(|(_, _, names)| names.as_slice()).unwrap_or_default();
        if names.is_empty() {
            let text = if self.filter.is_empty() { "No key." } else { "No key passes the filter." };
            ui.label(RichText::new(text).color(palette.text_faint));
        }
        names.iter().take(KEY_LIMIT).cloned().collect()
    }

    /// The note of the keys that pass the filter and have no row.
    pub(super) fn more(&self, ui: &mut Ui, palette: &Palette) {
        let count = self.names.as_ref().map_or(0, |(_, _, names)| names.len());
        if let Some(more) = count.checked_sub(KEY_LIMIT).filter(|more| *more > 0) {
            let text = format!("and {} more: the filter finds them", format::count(more as u64, "key"));
            ui.label(RichText::new(text).color(palette.text_muted));
        }
    }
}
