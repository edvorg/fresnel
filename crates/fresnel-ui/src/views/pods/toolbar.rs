//! The bar above the Pod table: the count of rows, the state of the rows and the filters.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::containers::menu::{MenuButton, MenuConfig};
use egui::{Align, Frame, Layout, Margin, PopupCloseBehavior, RichText, ScrollArea, TextEdit, Ui};

use super::rows::{PodFilter, PodModel};
use crate::format;
use crate::theme::{BAR_HEIGHT, Palette};
use crate::widgets;
use crate::widgets::table::{Progress, Table};

pub(super) fn show(ui: &mut Ui, palette: &Palette, filter: &mut PodFilter, table: &Table<PodModel>) {
    Frame::new().inner_margin(Margin::symmetric(12, 0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(BAR_HEIGHT);
            ui.label(RichText::new("Pods").size(15.0).strong());
            let count = ui.label(RichText::new(count_text(table)).color(palette.text_muted));
            if let Some(time) = table.compute_time() {
                count.on_hover_ui(|ui| {
                    ui.label(format!("The worker thread computed the rows in {}.", format::short_time(time)));
                });
            }
            if table.selection().len() > 1 {
                let selected = format!("{} selected", format::thousands(table.selection().len() as u64));
                ui.label(RichText::new(selected).color(palette.accent));
            }
            match table.progress() {
                Progress::Current => {}
                Progress::Behind => {
                    ui.label(RichText::new("updating…").color(palette.text_faint));
                }
                Progress::Failed(reason) => {
                    let text = format!("The rows are not current: the worker thread {reason}");
                    ui.label(RichText::new(text).color(palette.error));
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let field = TextEdit::singleline(&mut filter.text).hint_text("Filter").desired_width(200.0);
                widgets::name(&ui.add(field), "Filter pods");
                namespaces(ui, filter, table);
                let problems = ui.toggle_value(&mut filter.problems, "Problems");
                problems.on_hover_text("Shows only the pods that are not healthy");
            });
        });
    });
}

/// "56 of 1,204 pods": the rows after the filters, and the pods of the selected clusters.
fn count_text(table: &Table<PodModel>) -> String {
    let Some(rows) = table.rows() else {
        return String::new();
    };
    let (shown, total) = (rows.rows.len() as u64, rows.total as u64);
    if shown == total {
        format::count(total, "pod")
    } else {
        format!("{} of {}", format::thousands(shown), format::count(total, "pod"))
    }
}

/// The menu of the namespace filter. It lists each namespace with a pod, and each
/// namespace of the filter: a namespace whose last pod went away stays in the menu, so the
/// user can remove it from the filter.
fn namespaces(ui: &mut Ui, filter: &mut PodFilter, table: &Table<PodModel>) {
    let with_pods = table.rows().map(|rows| rows.summary.namespaces.as_slice()).unwrap_or_default();
    let listed: BTreeSet<&Arc<str>> = with_pods.iter().chain(&filter.namespaces).collect();
    let title = match filter.namespaces.len() {
        0 => "All namespaces".to_owned(),
        1 => filter.namespaces.first().map_or_else(String::new, |only| format!("Namespace: {only}")),
        more => format!("{more} namespaces"),
    };
    // The menu stays open at a click: the user selects more than one namespace.
    let config = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
    let mut selected = filter.namespaces.clone();
    MenuButton::new(title).config(config).ui(ui, |ui| {
        if ui.button("All namespaces").clicked() {
            selected.clear();
        }
        ui.separator();
        ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for namespace in listed {
                let mut on = selected.contains(namespace);
                if ui.checkbox(&mut on, &**namespace).changed() {
                    if on {
                        selected.insert(Arc::clone(namespace));
                    } else {
                        selected.remove(namespace);
                    }
                }
            }
        });
    });
    filter.namespaces = selected;
}
