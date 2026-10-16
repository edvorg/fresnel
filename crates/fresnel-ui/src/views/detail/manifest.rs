//! The tab Manifest of the panel (plan section 12.9): the YAML of the object as its server
//! gives it, read-only, with no `managedFields` until the user asks for them. Its button Edit
//! opens the editor of the object (`views/editor`).
//!
//! The core reads the manifest, and the tab asks for it with a command: one time for each
//! version of the object in its store, so the text follows the object. A deleted object
//! keeps its last text. The tab lays out only the lines on the screen, so a manifest of
//! many lines costs no more than a short one (plan section 12.9: virtual rows).

use std::ops::Range;
use std::sync::Arc;

use egui::{Label, RichText, ScrollArea, Ui};
use fresnel_core::app::{Command, ManifestRequest, ManifestState};
use fresnel_core::manifest::{ManagedFields, Manifest};

use super::fields::note;
use crate::format::short;
use crate::shell::Scene;
use crate::theme::{self, Palette, SMALL};
use crate::views::act::Gate;
use crate::widgets;

/// The most characters of one line that the tab lays out. A value of a ConfigMap can be
/// one line of megabytes.
const LINE_LIMIT: usize = 2000;

/// The object of the tab: its request, and its version in the store. `None` as the version
/// of an object that left its store: the tab asks for it no more. `edit` is the gate of the
/// editor of the object.
pub(super) struct Target<'a> {
    pub(super) request: ManifestRequest,
    pub(super) version: Option<&'a str>,
    pub(super) edit: Gate,
}

#[derive(Default)]
pub(super) struct ManifestTab {
    managed_fields: ManagedFields,
    /// The last request of the tab, with the version of the object in its store then.
    asked: Option<(ManifestRequest, Option<String>)>,
    /// The manifest on the screen, with the range of each of its lines.
    shown: Option<(ManifestRequest, Arc<Manifest>, Vec<Range<usize>>)>,
}

impl ManifestTab {
    /// What the tab shows of `managedFields`.
    pub(super) fn managed_fields(&self) -> ManagedFields {
        self.managed_fields
    }

    /// Draws the tab. Gives true for a click on Edit.
    pub(super) fn show(&mut self, ui: &mut Ui, palette: &Palette, scene: &mut Scene<'_>, target: Target<'_>) -> bool {
        let Target { request, version, edit } = target;
        let wanted = (request.clone(), version.map(str::to_owned));
        let (mut again, mut edited) = (false, false);
        ui.horizontal(|ui| {
            let mut button = ui.add_enabled(!edit.is_closed(), egui::Button::new(RichText::new("Edit…").size(SMALL)));
            if let Some(hint) = edit.hint() {
                button = button.on_hover_text(&hint).on_disabled_hover_text(hint);
            }
            widgets::name(&button, "Edit the manifest");
            edited = button.clicked();
            let mut kept = self.managed_fields == ManagedFields::Kept;
            if ui.checkbox(&mut kept, "managedFields").on_hover_text("Shows which client wrote each field").changed() {
                self.managed_fields = if kept { ManagedFields::Kept } else { ManagedFields::Removed };
            }
            let button = ui.small_button("Read again");
            widgets::name(&button, "Read the manifest again");
            again = button.clicked();
            let read = self.shown.as_ref().and_then(|(_, manifest, _)| manifest.version.as_deref());
            if let Some(read) = read {
                ui.label(RichText::new(format!("version {read}")).size(SMALL).color(palette.text_faint));
            }
        });
        let last = scene.view.manifest.as_ref().filter(|read| read.request == request);
        // A new version waits for the end of the read in progress: a new request would end
        // it, and an object that changes faster than a read would never show. A deleted
        // object has no version in its store, and a read of it would fail.
        let reading = last.is_some_and(|read| matches!(read.state, ManifestState::Reading));
        let newer = self.asked.as_ref() != Some(&wanted) && !(reading && self.asked_for(&request));
        if version.is_some() && (again || newer) && request.managed_fields == self.managed_fields {
            scene.commands.push(Command::ReadManifest(request.clone()));
            self.asked = Some(wanted);
        }
        match last.map(|read| &read.state) {
            Some(ManifestState::Done(manifest)) => self.keep(&request, manifest),
            Some(ManifestState::Failed(error)) => note(ui, palette.error, &format!("The read failed: {error}")),
            Some(ManifestState::Reading) | None if version.is_some() => {
                note(ui, palette.text_muted, &format!("The manifest is read from {}…", request.cluster));
            }
            Some(ManifestState::Reading) | None => {}
        }
        if version.is_none() {
            note(ui, palette.warn, "The object left its store. The tab shows the last manifest that it read.");
        }
        self.lines(ui, palette, &request);
        edited
    }

    /// True when the last request of the tab was for this object in this form.
    fn asked_for(&self, request: &ManifestRequest) -> bool {
        self.asked.as_ref().is_some_and(|(asked, _)| asked == request)
    }

    /// Takes a manifest that the core gave. The lines of the same text are known.
    fn keep(&mut self, request: &ManifestRequest, manifest: &Arc<Manifest>) {
        if self.shown.as_ref().is_some_and(|(_, shown, _)| Arc::ptr_eq(shown, manifest)) {
            return;
        }
        self.shown = Some((request.clone(), Arc::clone(manifest), lines(&manifest.text)));
    }

    /// Draws the lines of the manifest on the screen, if it is the manifest of the request.
    fn lines(&self, ui: &mut Ui, palette: &Palette, request: &ManifestRequest) {
        let Some((_, manifest, lines)) = self.shown.as_ref().filter(|(shown, _, _)| shown == request) else {
            return;
        };
        ui.add_space(4.0);
        let font = theme::mono();
        let height = ui.fonts_mut(|fonts| fonts.row_height(&font)) + ui.spacing().item_spacing.y;
        let area = ScrollArea::both().id_salt("manifest").auto_shrink(false);
        area.show_rows(ui, height, lines.len(), |ui, shown| {
            for range in lines.get(shown).unwrap_or_default() {
                let line = manifest.text.get(range.clone()).unwrap_or_default();
                let text = RichText::new(short(line, LINE_LIMIT)).font(font.clone()).color(palette.text);
                ui.add(Label::new(text).extend());
            }
        });
    }
}

/// The range of each line of a text, with no line break.
fn lines(text: &str) -> Vec<Range<usize>> {
    let mut start = 0;
    let mut lines = Vec::new();
    for (end, _) in text.match_indices('\n') {
        lines.push(start..end);
        start = end + 1;
    }
    if start < text.len() {
        lines.push(start..text.len());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_line_has_its_range_and_the_last_line_break_ends_the_last_line() {
        let text = "a: 1\nb:\n  - x\n";
        let found: Vec<&str> = lines(text).into_iter().filter_map(|range| text.get(range)).collect();
        assert_eq!(found, ["a: 1", "b:", "  - x"]);
        assert_eq!(lines("one"), vec![0..3]);
        assert_eq!(lines(""), Vec::<Range<usize>>::new());
    }
}
