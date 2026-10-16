//! The bar of a tab of a create: the target cluster and namespace, Create… and Apply…, the
//! undo, the redo and the search, and the state of the check.

use egui::{Button, ComboBox, Layout, RichText, TextEdit, Ui};
use fresnel_core::cluster::ClusterId;

use super::plan::Way;
use crate::shell::Scene;
use crate::theme::{self, SMALL};
use crate::views::editor::bar::Asked;
use crate::widgets;

/// What the bar shows besides the target.
pub(super) struct Parts {
    pub(super) undo: bool,
    pub(super) redo: bool,
    /// Whether the text can go now, or why not.
    pub(super) run: Result<(), String>,
    /// The state of the check, with true for an error.
    pub(super) check: (String, bool),
}

/// What the user asked for in the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ask {
    /// An act on the text or the search, as in the bar of an editor.
    Text(Asked),
    /// Opens or closes the search.
    Find,
    Run(Way),
}

/// Draws the bar. The cluster menu lists each selected and connected cluster.
pub(super) fn show(
    ui: &mut Ui,
    scene: &Scene<'_>,
    (cluster, namespace): (&mut Option<ClusterId>, &mut String),
    parts: &Parts,
) -> Vec<Ask> {
    let palette = theme::palette(ui);
    let mut asked = Vec::new();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Create or apply YAML").strong());
        ui.label(RichText::new("in").color(palette.text_muted));
        let shown = cluster.as_ref().map_or("no cluster", ClusterId::as_str).to_owned();
        let menu = ComboBox::from_id_salt("cluster of the yaml").selected_text(shown).show_ui(ui, |ui| {
            for row in scene.view.clusters.iter().filter(|row| row.selected && row.data.is_some()) {
                let option = ui.selectable_value(cluster, Some(row.id.clone()), row.id.as_str());
                widgets::name(&option, &format!("Target {}", row.id));
            }
        });
        widgets::name(&menu.response, "Target cluster of the YAML");
        if let Some(cluster) = cluster.as_ref() {
            widgets::dot(ui, scene.colors.color(cluster, palette));
        }
        ui.label(RichText::new("namespace").color(palette.text_muted));
        let field = ui.add(TextEdit::singleline(namespace).desired_width(140.0));
        widgets::name(&field, "Target namespace of the YAML");
        for way in [Way::Create, Way::Apply] {
            let button =
                ui.add_enabled(parts.run.is_ok(), Button::new(RichText::new(format!("{}…", way.verb())).size(SMALL)));
            widgets::name(&button, &format!("{} the documents", way.verb()));
            if let Err(reason) = &parts.run {
                button.clone().on_disabled_hover_text(reason);
            }
            if button.clicked() {
                asked.push(Ask::Run(way));
            }
        }
        let mut button = |ui: &mut Ui, (text, enabled): (&str, bool), ask: Asked| {
            let response = ui.add_enabled(enabled, Button::new(RichText::new(text).size(SMALL)));
            widgets::name(&response, text);
            if response.clicked() {
                asked.push(Ask::Text(ask));
            }
        };
        button(ui, ("Undo", parts.undo), Asked::Undo);
        button(ui, ("Redo", parts.redo), Asked::Redo);
        let find = ui.add(Button::new(RichText::new("Find").size(SMALL)));
        widgets::name(&find, "Find in the text");
        if find.clicked() {
            asked.push(Ask::Find);
        }
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            let (state, error) = &parts.check;
            let color = if *error { palette.error } else { palette.text_muted };
            ui.label(RichText::new(state).size(SMALL).color(color));
        });
    });
    if let Err(reason) = &parts.run {
        ui.label(RichText::new(reason).size(SMALL).color(palette.text_muted));
    }
    asked
}
