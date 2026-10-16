//! The dialog before the save of an editor (plan sections 11.2 and 12.9): the object as
//! cluster, namespace, kind and name, the version of the read, and the diff of the read and
//! the object as the server would store it, from the dry run of the save.
//!
//! The dialog holds the save that the dry run checked, so the save that goes is the one that
//! the user saw. A read-only cluster and the confirm rule `type-name` keep its button off,
//! in each frame, as before each other change.

use std::sync::Arc;

use egui::{Align, Color32, Context, Id, Key, Layout, Modal, RichText};
use fresnel_core::app::{CoreView, EditSave};
use fresnel_core::manifest::EditText;

use super::diff::{self, Row};
use crate::format::short;
use crate::theme;
use crate::views::act::TypedName;
use crate::widgets::{self, dot};

/// The width of the dialog.
const WIDTH: f32 = 760.0;
/// The height of the diff before it scrolls.
const DIFF_HEIGHT: f32 = 360.0;

/// What the user chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Answer {
    Save,
    Cancel,
}

pub(super) struct SaveDialog {
    /// The save that the dry run checked.
    save: EditSave,
    rows: Vec<Row>,
    typed: TypedName,
}

impl SaveDialog {
    /// The dialog of a save whose dry run gave `after`.
    pub(super) fn new(save: EditSave, after: &Arc<EditText>) -> Self {
        let rows = diff::rows(&save.base.text, &after.text);
        Self { save, rows, typed: TypedName::default() }
    }

    /// The save of the dialog.
    pub(super) fn save(&self) -> &EditSave {
        &self.save
    }

    /// Draws the dialog. `color` is the color of the cluster.
    pub(super) fn show(&mut self, ctx: &Context, color: Color32, view: &CoreView) -> Option<Answer> {
        let modal = Modal::new(Id::new("save editor")).show(ctx, |ui| {
            ui.set_width(WIDTH);
            self.contents(ui, color, view)
        });
        let escape = ctx.input(|input| input.key_pressed(Key::Escape));
        modal.inner.or(escape.then_some(Answer::Cancel))
    }

    fn contents(&mut self, ui: &mut egui::Ui, color: Color32, view: &CoreView) -> Option<Answer> {
        let palette = theme::palette(ui);
        let target = &self.save.editor.target;
        let (kind, key) = (target.kind.info().name, &target.key);
        ui.label(RichText::new(format!("Save this {kind}?")).heading());
        ui.horizontal_wrapped(|ui| {
            dot(ui, color);
            let parts = [
                ("Cluster", target.cluster.as_str()),
                ("Namespace", &*key.namespace),
                ("Kind", kind),
                ("Name", &*key.name),
            ];
            for (label, value) in parts.into_iter().filter(|(_, value)| !value.is_empty()) {
                ui.label(RichText::new(label).color(palette.text_muted));
                ui.label(RichText::new(short(value, 120)).strong());
            }
        });
        let version = self.save.base.version.as_deref().unwrap_or("none");
        let note = format!(
            "The server checked the text in a dry run. The diff shows the object as it would store it. The save \
             replaces version {version} of the read, and the server refuses it when the object changed since."
        );
        ui.add(egui::Label::new(RichText::new(note).color(palette.text_muted)).wrap());
        ui.add_space(6.0);
        if self.rows.is_empty() {
            ui.label(RichText::new("The server would store no change.").color(palette.warn));
        } else {
            egui::Frame::new().fill(palette.bg_sunken).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                diff::show(ui, &self.rows, DIFF_HEIGHT);
            });
        }
        ui.add_space(8.0);
        let typed = self.typed.show(ui, palette, view, std::iter::once(&target.cluster));
        let refusal = view.config.refusal(&target.cluster).or(typed);
        if let Some(reason) = &refusal {
            ui.label(RichText::new(reason).color(palette.warn));
        }
        let mut answer = None;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let enabled = refusal.is_none() && !self.rows.is_empty();
            let save = ui.add_enabled(enabled, egui::Button::new(RichText::new("Save").strong()));
            widgets::name(&save, "Save the manifest");
            if save.clicked() {
                answer = Some(Answer::Save);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(Answer::Cancel);
            }
        });
        answer
    }
}
