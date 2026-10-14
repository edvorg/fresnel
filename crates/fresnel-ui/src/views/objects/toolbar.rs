//! The bar above the table of a kind: the count of rows, the state of the rows, the
//! filters and the choice of the columns. The namespace filter is in the top bar: it is
//! one filter for each page.

use egui::containers::menu::{MenuButton, MenuConfig};
use egui::{Align, Frame, Layout, Margin, PopupCloseBehavior, RichText, TextEdit, Ui};

use super::filter::ObjectFilter;
use super::{Nouns, ObjectModel};
use crate::format;
use crate::theme::{BAR_HEIGHT, Palette};
use crate::widgets;
use crate::widgets::table::{Progress, Table};

/// Draws the bar. `focus` asks for the focus of the text filter, one time.
pub(super) fn show<M: ObjectModel>(
    ui: &mut Ui,
    palette: &Palette,
    nouns: &Nouns,
    filter: &mut ObjectFilter,
    table: &mut Table<M>,
    focus: &mut bool,
) {
    Frame::new().inner_margin(Margin::symmetric(12, 0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(BAR_HEIGHT);
            ui.label(RichText::new(nouns.title).size(15.0).strong());
            let count = ui.label(RichText::new(count_text(table, nouns)).color(palette.text_muted));
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
                let field = ui.add(field);
                widgets::name(&field, &format!("Filter {}", nouns.many));
                if std::mem::take(focus) {
                    field.request_focus();
                }
                columns(ui, table);
                let problems = ui.toggle_value(&mut filter.problems, "Problems");
                problems.on_hover_text(format!("Shows only the {} that are not healthy", nouns.many));
            });
        });
    });
}

/// "56 of 1,204 pods": the rows after the filters, and the objects of the selected clusters.
fn count_text<M: ObjectModel>(table: &Table<M>, nouns: &Nouns) -> String {
    let Some(rows) = table.rows() else {
        return String::new();
    };
    let (shown, total) = (rows.rows.len() as u64, rows.total as u64);
    if shown == total { nouns.count(total) } else { format!("{} of {}", format::thousands(shown), nouns.count(total)) }
}

/// The menu of the columns: a switch for each column but the first one.
fn columns<M: ObjectModel>(ui: &mut Ui, table: &mut Table<M>) {
    let config = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
    let mut chosen = Vec::new();
    MenuButton::new("Columns").config(config).ui(ui, |ui| {
        for (index, title, shown) in table.column_choices() {
            let mut on = shown;
            if ui.checkbox(&mut on, title).changed() {
                chosen.push((index, on));
            }
        }
    });
    for (index, shown) in chosen {
        table.show_column(index, shown);
    }
}
