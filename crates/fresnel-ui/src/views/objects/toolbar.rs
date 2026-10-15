//! The bar above the table of a kind: the count of rows, the state of the rows, the bulk
//! actions on the selected rows, the button of a new object, the filters and the choice of
//! the columns. The namespace filter is in the top bar: it is one filter for each page.

use egui::containers::menu::{MenuButton, MenuConfig};
use egui::{Align, Frame, Id, Label, Layout, Margin, PopupCloseBehavior, RichText, TextEdit, Ui, UiBuilder};
use fresnel_core::commands::{self, CommandId};
use fresnel_core::kinds::{Action, Describe, Kind};

use super::filter::ObjectFilter;
use super::{Nouns, ObjectModel};
use crate::format;
use crate::theme::{BAR_HEIGHT, Palette};
use crate::views::act::Gate;
use crate::views::detail::actions;
use crate::widgets;
use crate::widgets::table::{Progress, Table};

/// The command that makes a new object of a kind, for a button of its bar. Only a Secret
/// has one (plan section 12.2).
fn create_command(kind: Kind) -> Option<CommandId> {
    (kind == Kind::Secret).then_some(CommandId::NewSecret)
}

/// What the user chose in the bar.
pub(super) enum Picked {
    /// The command of the button of a new object.
    Create(CommandId),
    /// A bulk action on the selected rows.
    Bulk(Action),
}

/// Draws the bar. `focus` asks for the focus of the text filter, one time. `bulk` has the
/// gate of each bulk action on the selected rows. Gives what the user clicked.
pub(super) fn show<M: ObjectModel>(
    ui: &mut Ui,
    palette: &Palette,
    nouns: &Nouns,
    filter: &mut ObjectFilter,
    table: &mut Table<M>,
    focus: &mut bool,
    bulk: &[(Action, Gate)],
) -> Option<Picked> {
    let mut chosen = None;
    Frame::new().inner_margin(Margin::symmetric(12, 0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(BAR_HEIGHT);
            // The id of a widget counts the widgets before it. A note before the filters that
            // comes and goes, as "updating…" for a slow worker, thus gave them new ids: the
            // field lost the focus and the next keys, and an open menu closed. These are fixed.
            let id = Id::new(("filters of the bar", nouns.title));
            ui.scope_builder(UiBuilder::new().id(id).layout(Layout::right_to_left(Align::Center)), |ui| {
                let field = TextEdit::singleline(&mut filter.text).hint_text("Filter").desired_width(200.0);
                let field = ui.add(field);
                widgets::name(&field, &format!("Filter {}", nouns.many));
                if std::mem::take(focus) {
                    field.request_focus();
                }
                columns(ui, table);
                let problems = ui.toggle_value(&mut filter.problems, "Problems");
                problems.on_hover_text(format!("Shows only the {} that are not healthy", nouns.many));
                // The rest of the bar takes the room that the filters leave, so a long title
                // of a kind cuts its texts and does not run under the buttons.
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    chosen = state(ui, palette, nouns, table, bulk);
                });
            });
        });
    });
    chosen
}

/// The left part of the bar: the title, the count of rows, the bulk menu, the button of a
/// new object and the state of the rows.
fn state<M: ObjectModel>(
    ui: &mut Ui,
    palette: &Palette,
    nouns: &Nouns,
    table: &Table<M>,
    bulk: &[(Action, Gate)],
) -> Option<Picked> {
    let mut chosen = None;
    ui.add(Label::new(RichText::new(nouns.title).size(15.0).strong()).truncate());
    let count = ui.add(Label::new(RichText::new(count_text(table, nouns)).color(palette.text_muted)).truncate());
    if let Some(time) = table.compute_time() {
        count.on_hover_ui(|ui| {
            ui.label(format!("The worker thread computed the rows in {}.", format::short_time(time)));
        });
    }
    if table.selection().len() > 1 {
        chosen = bulk_menu(ui, palette, table.selection().len(), bulk).map(Picked::Bulk);
    }
    let create = create_command(<M::Object as Describe>::ID).and_then(commands::entry);
    if let Some(entry) = create
        && ui.small_button(entry.label).clicked()
    {
        chosen = Some(Picked::Create(entry.id));
    }
    match table.progress() {
        Progress::Current => {}
        Progress::Behind => {
            ui.label(RichText::new("updating…").color(palette.text_faint));
        }
        Progress::Failed(reason) => {
            let text = format!("The rows are not current: the worker thread {reason}");
            ui.add(Label::new(RichText::new(text).color(palette.error)).truncate());
        }
    }
    chosen
}

/// The menu of the selected rows with a button for each bulk action and its gate. A menu
/// needs no more room in the bar, and a line of its own would move the rows under the
/// pointer when a click with Cmd adds the second row. Gives the action of a click.
fn bulk_menu(ui: &mut Ui, palette: &Palette, count: usize, bulk: &[(Action, Gate)]) -> Option<Action> {
    let selected = RichText::new(format!("{} selected", format::thousands(count as u64))).color(palette.accent);
    let config = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
    let (_, inner) = MenuButton::new(selected).config(config).ui(ui, |ui| bulk_buttons(ui, bulk));
    inner.and_then(|inner| inner.inner)
}

/// A button for each bulk action, with its gate. Gives the action of a click.
fn bulk_buttons(ui: &mut Ui, bulk: &[(Action, Gate)]) -> Option<Action> {
    let mut chosen = None;
    if bulk.is_empty() {
        ui.label("This kind has no bulk action.");
    }
    for (action, gate) in bulk {
        let button = ui.add_enabled(!gate.is_closed(), egui::Button::new(actions::label(*action)));
        widgets::name(&button, &format!("{} the selected", action.label()));
        let button = match gate.hint() {
            Some(hint) => button.on_hover_text(&hint).on_disabled_hover_text(hint),
            None => button,
        };
        if button.clicked() {
            chosen = Some(*action);
            ui.close();
        }
    }
    chosen
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
