//! The groups of the settings table: one row for each setting, with the control of its
//! form, its bounds and a button for its default. The table of the core gives each row, so
//! the view and the reader of the file have one list.

use egui::{ComboBox, Grid, RichText, Ui};
use fresnel_core::config::table::{self, Form, Group, Number, Spec};
use fresnel_core::config::{Config, EditValue, amount, is_time_zone};

use super::draft::{Draft, Path, lines, path};
use crate::theme::{Palette, SMALL};
use crate::widgets;

/// Draws the rows of a group, with no row for a setting that has no use on this system.
pub(super) fn group(ui: &mut Ui, palette: &Palette, config: &Config, draft: &mut Draft, group: Group) {
    if group.at_start() {
        let note = "These settings act at the next start of the app.";
        ui.label(RichText::new(note).size(SMALL).color(palette.text_muted));
    }
    Grid::new(("settings group", group.name())).num_columns(3).spacing([16.0, 6.0]).show(ui, |ui| {
        for spec in table::of_group(group).filter(|spec| table::applies(spec)) {
            let key = path(&[group.name(), spec.key]);
            ui.label(spec.label).on_hover_text(format!("[{}] {}", group.name(), spec.key));
            ui.horizontal(|ui| control(ui, palette, config, draft, (spec, &key)));
            draft.reset(ui, (&key, spec.label), config.set(spec.id).is_some());
            ui.end_row();
        }
    });
}

/// The value of a setting in the file, as a change would write it.
fn current(config: &Config, spec: &Spec) -> EditValue {
    match spec.form {
        Form::Number(_) => EditValue::Number(i64::try_from(config.number(spec.id)).unwrap_or(i64::MAX)),
        Form::Choice { .. } | Form::Text(_) => EditValue::Text(config.text(spec.id).to_owned()),
        Form::Flag(_) => EditValue::Flag(config.flag(spec.id)),
        Form::List(_) => EditValue::List(config.list(spec.id)),
    }
}

fn control(ui: &mut Ui, palette: &Palette, config: &Config, draft: &mut Draft, (spec, key): (&Spec, &Path)) {
    let now = current(config, spec);
    let value = draft.get(key).filter(|value| **value != EditValue::Remove).cloned().unwrap_or_else(|| now.clone());
    match spec.form {
        Form::Number(number) => {
            let text = match &now {
                EditValue::Number(value) => value.to_string(),
                _ => String::new(),
            };
            draft.field(ui, palette, (key, spec.label, false), &text, |text| read_number(text, number));
            let range = format!("{} to {}", amount(number.min, number.unit), amount(number.max, number.unit));
            ui.label(RichText::new(range).size(SMALL).color(palette.text_faint));
        }
        Form::Choice { options, .. } => {
            let EditValue::Text(chosen) = &value else { return };
            let mut picked = chosen.clone();
            let combo = ComboBox::from_id_salt(key).selected_text(picked.as_str()).show_ui(ui, |ui| {
                for option in options {
                    ui.selectable_value(&mut picked, (*option).to_owned(), *option);
                }
            });
            widgets::name(&combo.response, spec.label);
            if picked != *chosen {
                draft.set(key, EditValue::Text(picked), &now);
            }
        }
        Form::Flag(_) => {
            let EditValue::Flag(mut on) = value else { return };
            let before = on;
            let check = ui.checkbox(&mut on, "");
            widgets::name(&check, spec.label);
            if on != before {
                draft.set(key, EditValue::Flag(on), &now);
            }
        }
        Form::Text(_) => {
            let EditValue::Text(text) = &now else { return };
            draft.field(ui, palette, (key, spec.label, false), text, |text| match text.trim() {
                zone if is_time_zone(zone) => Ok(EditValue::Text(zone.to_owned())),
                zone => Err(format!("no time zone {zone}: local, UTC or a name such as Europe/Amsterdam")),
            });
        }
        Form::List(_) => {
            let EditValue::List(items) = &now else { return };
            let read = |text: &str| Ok(EditValue::List(lines(text)));
            draft.field(ui, palette, (key, spec.label, true), &items.join("\n"), read);
            ui.label(RichText::new("one on each line").size(SMALL).color(palette.text_faint));
        }
    }
}

/// A whole number inside its bounds. The view refuses what the reader would clamp, so the
/// file gets the number that the user sees.
fn read_number(text: &str, number: Number) -> Result<EditValue, String> {
    let range = format!("a whole number from {} to {}", number.min, number.max);
    let value: u64 = text.trim().parse().map_err(|_| range.clone())?;
    if !(number.min..=number.max).contains(&value) {
        return Err(range);
    }
    i64::try_from(value).map(EditValue::Number).map_err(|_| range)
}

#[cfg(test)]
mod tests {
    use fresnel_core::config::table::Unit;

    use super::*;

    #[test]
    fn a_number_of_the_view_is_inside_its_bounds() {
        let number = Number { default: 15, min: 10, max: 120, unit: Unit::Seconds, period: true };
        assert_eq!(read_number(" 30 ", number), Ok(EditValue::Number(30)));
        for bad in ["9", "121", "-1", "ten", ""] {
            assert_eq!(read_number(bad, number), Err("a whole number from 10 to 120".to_owned()), "{bad}");
        }
    }
}
