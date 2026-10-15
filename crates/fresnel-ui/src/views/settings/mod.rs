//! The settings view (plan section 12.5): each setting of `config.toml` with its control, the
//! options of each cluster, the cluster groups and the keys. Save sends the changes to the
//! core, which writes them into the file and keeps its comments (`Command::EditSettings`).
//! The view shows each problem of the file with its line and column, and it saves nothing
//! while the app cannot read the file: the core would refuse it too.
//!
//! - `draft`: the changes that the file does not have yet. `fixed`: the groups of the table.
//! - `clusters`: the options of each cluster and the groups. `keys`: the key of each command.

mod clusters;
mod draft;
mod fixed;
mod keys;

use egui::{Align, CollapsingHeader, Frame, Layout, RichText, ScrollArea, Ui};
use fresnel_core::app::Command;
use fresnel_core::config::table::Group;
use fresnel_core::config::{FileRead, FileState};

use crate::format;
use crate::shell::Scene;
use crate::theme::{self, PAD, Palette};
use crate::widgets::{self, tone::Tone};
use draft::Draft;

/// The page of the settings, with the changes that the user did not save yet.
#[derive(Debug, Default)]
pub(crate) struct SettingsPage {
    draft: Draft,
    /// The number of the save that the view sent, until the core gives its result.
    sent: Option<u64>,
    /// The end of the last save of the view.
    note: Option<(Tone, String)>,
    /// The name of a new cluster group.
    new_group: String,
}

impl SettingsPage {
    pub(crate) fn show(&mut self, ui: &mut Ui, scene: &mut Scene<'_>) {
        let palette = theme::palette(ui);
        let view = scene.view;
        self.follow(&view.config_file);
        Frame::new().inner_margin(PAD).show(ui, |ui| {
            ui.label(RichText::new("Settings").heading().strong());
            let path = view.config_file.path.display().to_string();
            ui.label(RichText::new(path).monospace().color(palette.text_muted));
            let refusal = state(ui, palette, &view.config_file.read);
            self.bar(ui, palette, scene, refusal.is_none());
            ui.add_space(4.0);
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.add_enabled_ui(refusal.is_none(), |ui| self.sections(ui, palette, scene));
            });
        });
    }

    /// Takes the result of the save that the view sent. A good save empties the draft: the
    /// controls then show the values of the file, which the core read again.
    fn follow(&mut self, file: &FileState) {
        let Some(sent) = self.sent else {
            return;
        };
        let Some(result) = file.save.as_ref().filter(|save| save.number >= sent).and_then(|save| save.result.clone())
        else {
            return;
        };
        self.sent = None;
        self.note = Some(match result {
            Ok(()) => {
                self.draft.clear();
                (Tone::Ok, "Saved into the file.".to_owned())
            }
            Err(error) => (Tone::Error, format!("Not saved: {error}")),
        });
    }

    /// The count of the changes, Save and Discard, and the end of the last save.
    fn bar(&mut self, ui: &mut Ui, palette: &Palette, scene: &mut Scene<'_>, readable: bool) {
        ui.horizontal(|ui| {
            let count = self.draft.count();
            let problem = self.draft.problem();
            let can = readable && count > 0 && problem.is_none() && self.sent.is_none();
            let save = ui.add_enabled(can, egui::Button::new(RichText::new("Save").strong()));
            widgets::name(&save, "Save the settings");
            if save.clicked() {
                let number = scene.view.config_file.save.as_ref().map_or(1, |save| save.number + 1);
                scene.commands.push(Command::EditSettings(self.draft.edits()));
                (self.sent, self.note) = (Some(number), None);
            }
            if ui.add_enabled(count > 0 || problem.is_some(), egui::Button::new("Discard")).clicked() {
                (self.draft, self.note) = (Draft::default(), None);
            }
            let text = match (&problem, self.sent) {
                (Some(problem), _) => problem.clone(),
                (None, Some(_)) => "The app writes the file…".to_owned(),
                (None, None) if count == 0 => "No change.".to_owned(),
                (None, None) => format!("{} to save.", format::count(count as u64, "change")),
            };
            ui.label(RichText::new(text).color(palette.text_muted));
            if let Some((tone, note)) = &self.note {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(note).color(tone.color(palette)));
                });
            }
        });
    }

    fn sections(&mut self, ui: &mut Ui, palette: &Palette, scene: &Scene<'_>) {
        let view = scene.view;
        for group in Group::ALL {
            let open = matches!(group, Group::Appearance | Group::Table | Group::Safety | Group::Notifications);
            CollapsingHeader::new(group.title()).default_open(open).show(ui, |ui| {
                fixed::group(ui, palette, &view.config, &mut self.draft, group);
            });
        }
        CollapsingHeader::new("Clusters").default_open(true).show(ui, |ui| {
            clusters::options(ui, palette, view, &mut self.draft);
        });
        CollapsingHeader::new("Cluster groups").default_open(false).show(ui, |ui| {
            clusters::groups(ui, palette, view, &mut self.draft, &mut self.new_group);
        });
        CollapsingHeader::new("Keys").default_open(false).show(ui, |ui| {
            keys::show(ui, palette, &view.config.keys, &mut self.draft);
        });
    }
}

/// Draws the state of the last read of the file and each of its problems. Gives why the
/// view cannot save, if it cannot.
fn state(ui: &mut Ui, palette: &Palette, read: &FileRead) -> Option<String> {
    let (tone, text) = match read {
        FileRead::NotYet => (Tone::Info, "The app reads the file…".to_owned()),
        FileRead::Absent => (Tone::Info, "No file: each setting has its default. Save makes the file.".to_owned()),
        FileRead::Read { problems, used: true } if problems.is_empty() => {
            (Tone::Ok, "Read with no problem.".to_owned())
        }
        FileRead::Read { problems, used: true } => (
            Tone::Warn,
            format!("Read with {}; the other keys are in use:", format::count(problems.len() as u64, "problem")),
        ),
        FileRead::Read { used: false, .. } | FileRead::Failed(_) => {
            (Tone::Error, "The app uses no setting of the file:".to_owned())
        }
    };
    ui.label(RichText::new(text).color(tone.color(palette)));
    if let FileRead::Read { problems, .. } = read {
        for problem in problems {
            ui.label(RichText::new(problem.to_string()).color(tone.color(palette)));
        }
    }
    let refusal = read.refusal().filter(|_| *read != FileRead::NotYet);
    if let Some(refusal) = &refusal {
        ui.label(RichText::new(refusal).color(palette.error).strong());
    }
    refusal.or_else(|| (*read == FileRead::NotYet).then(|| "The app did not read the file yet.".to_owned()))
}
