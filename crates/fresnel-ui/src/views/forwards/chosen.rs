//! The chosen forward of the Forwards tab: its target with the color of its cluster, its
//! state with the reason, its buttons, and its addresses, its pod, its counts, the age of the
//! data of its target and its last error. The list has one short line of each forward, so
//! the long texts are here.

use egui::{
    Align, Button, Color32, Label, Layout, OpenUrl, RichText, ScrollArea, TextStyle, TextWrapMode, Ui, WidgetText,
};
use fresnel_core::app::{Command, ForwardRow};
use fresnel_core::config::forwards::{forget_edit, save_edits};
use fresnel_core::forward::ForwardState;

use super::texts;
use crate::format::short;
use crate::shell::Scene;
use crate::theme::{self, Palette};
use crate::views::detail::fields::{self, VALUE_LIMIT};
use crate::views::operations::TEXT_LIMIT;
use crate::widgets::tone::Tone;
use crate::widgets::{self, DOT, dot};

/// The panel scrolls as one part: the dock can be short, and the buttons stay in reach.
pub(super) fn show(ui: &mut Ui, scene: &mut Scene<'_>, row: &ForwardRow) {
    let palette = theme::palette(ui);
    let view = &row.view;
    let cluster = &view.rule.cluster;
    ScrollArea::vertical().id_salt(("forward", row.id)).auto_shrink([false, true]).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(view.rule.target()).strong());
            ui.add_space(6.0);
            let alias = scene.view.config.alias(cluster).unwrap_or(cluster.as_str());
            cluster_name(ui, scene.colors.color(cluster, palette), RichText::new(alias).color(palette.text_muted));
        });
        let (state, tone) = texts::state(view, scene.clock);
        ui.horizontal_wrapped(|ui| {
            dot(ui, tone.color(palette));
            ui.add(Label::new(RichText::new(short(&state, TEXT_LIMIT)).color(tone.color(palette))).wrap());
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| buttons(ui, scene, row));
        ui.add_space(8.0);
        let toned = |ui: &mut Ui, label: &str, (text, tone): (String, Tone)| {
            line(ui, palette, label, RichText::new(short(&text, VALUE_LIMIT)).color(tone.color(palette)));
        };
        line(ui, palette, "Local address", RichText::new(texts::addresses(view)).font(theme::mono()));
        toned(ui, "Pod", texts::pod(view));
        toned(ui, "Connections", (texts::connections(view), Tone::Normal));
        toned(ui, "Bytes", (texts::bytes(view), Tone::Normal));
        toned(ui, "Target data", texts::data(view, scene.clock));
        toned(ui, "Last error", texts::last_error(view, scene.clock));
        let saved = if row.saved { "yes, in config.toml" } else { "no" };
        toned(ui, "Saved", (saved.to_owned(), Tone::Normal));
    });
}

/// The dot and the name of the cluster as one item of the wrapped title: the line breaks
/// before the dot, as a dot alone at the end of a line reads as a state. A nested
/// `horizontal` takes the rest of the line before it knows its width, so it would not break.
/// A name wider than the panel, as a context of EKS with no alias, ends in an ellipsis, and
/// the label shows it in full under the pointer.
fn cluster_name(ui: &mut Ui, color: Color32, name: RichText) {
    let gap = ui.spacing().item_spacing.x;
    let width = ui.max_rect().width() - DOT - gap;
    let galley = WidgetText::from(name).into_galley(ui, Some(TextWrapMode::Truncate), width, TextStyle::Body);
    let size = egui::vec2(DOT + gap + galley.size().x, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
        dot(ui, color);
        ui.label(galley);
    });
}

/// One line of the panel: the label at the left and the text, which wraps, at the right.
fn line(ui: &mut Ui, palette: &Palette, label: &str, text: RichText) {
    fields::field(ui, palette, label, |ui| {
        ui.add_space(1.0);
        ui.add(Label::new(text).wrap());
    });
}

/// The buttons of the chosen forward: copy and open its address, stop or start it, "retry
/// now", save or forget its rule, and remove it.
fn buttons(ui: &mut Ui, scene: &mut Scene<'_>, row: &ForwardRow) {
    let view = &row.view;
    let address = view.address();
    let target = view.rule.to_string();
    let button = |ui: &mut Ui, text: &str, enabled: bool| {
        let response = ui.add_enabled(enabled, Button::new(text).small());
        widgets::name(&response, &format!("{text}: {target}"));
        response.clicked()
    };
    if button(ui, "Copy address", address.is_some())
        && let Some(address) = address
    {
        ui.ctx().copy_text(address.to_string());
    }
    if button(ui, "Open in browser", address.is_some())
        && let Some(address) = address
    {
        ui.ctx().open_url(OpenUrl::new_tab(format!("http://{address}/")));
    }
    let stopped = matches!(view.state, ForwardState::Stopped | ForwardState::Failed(_));
    if stopped {
        if button(ui, "Start", true) {
            scene.commands.push(Command::RestartForward(row.id));
        }
    } else if button(ui, "Stop", true) {
        scene.commands.push(Command::StopForward(row.id));
    }
    let retries = matches!(view.state, ForwardState::Reconnecting { .. } | ForwardState::Degraded { .. });
    if button(ui, "Retry now", retries) {
        scene.commands.push(Command::RetryForward(row.id));
    }
    match row.saved {
        true if button(ui, "Forget", true) => {
            scene.commands.push(Command::EditSettings(vec![forget_edit(&view.rule)]));
        }
        false if button(ui, "Save", true) => scene.commands.push(Command::EditSettings(save_edits(&view.rule))),
        _ => {}
    }
    let removed = ui.add_enabled(!row.saved, Button::new("Remove").small());
    widgets::name(&removed, &format!("Remove: {target}"));
    let removed = removed.on_disabled_hover_text("A saved forward stays in the list: Forget it first.");
    if removed.clicked() {
        scene.commands.push(Command::RemoveForward(row.id));
    }
}
