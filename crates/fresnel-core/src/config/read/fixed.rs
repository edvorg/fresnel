//! The groups of the settings table: each key with its form, and each number inside its
//! bounds. A number outside them takes the nearest bound with a note, and a period or a time
//! limit of 0 is refused: the setting keeps its default.

use std::collections::BTreeMap;

use toml::Spanned;
use toml::de::DeValue;

use super::{Reader, integer};
use crate::config::table::{self, Form, Group, Number, Setting, Spec, Unit};
use crate::config::values::Value;

/// The most entries of a list, for example of folders. A list of a file is short; a long
/// one is a mistake, and each entry costs a look at the disk.
const LIST_LIMIT: usize = 32;
/// The longest text of a setting, such as a path or the name of a time zone.
const TEXT_LIMIT: usize = 1024;

impl Reader<'_> {
    /// The table of a group of the settings table.
    pub(super) fn fixed(&mut self, group: Group, value: &Spanned<DeValue<'_>>, values: &mut BTreeMap<Setting, Value>) {
        let Some(entries) = self.table_of(group.name(), value) else {
            return;
        };
        for (key, value) in entries.iter() {
            let name = key.get_ref().as_ref();
            let Some(spec) = table::find(group, name) else {
                self.problem(key.span(), format!("unknown key `{name}` in `{}`; the app ignores it", group.name()));
                continue;
            };
            if let Some(read) = self.value(spec, value) {
                values.insert(spec.id, read);
            }
        }
    }

    fn value(&mut self, spec: &Spec, value: &Spanned<DeValue<'_>>) -> Option<Value> {
        let path = format!("{}.{}", spec.group.name(), spec.key);
        match spec.form {
            Form::Number(number) => self.number(&path, number, value).map(Value::Number),
            Form::Choice { options, .. } => {
                let text = value.get_ref().as_str().filter(|text| options.contains(text));
                if text.is_none() {
                    let options: Vec<String> = options.iter().map(|option| format!("\"{option}\"")).collect();
                    self.problem(value.span(), format!("`{path}` is one of {}", options.join(", ")));
                }
                text.map(|text| Value::Text(text.to_owned()))
            }
            Form::Flag(_) => self.flag(&path, value).map(Value::Flag),
            Form::Text(_) => {
                let text = self.text(&path, value)?;
                match text_problem(spec.id, text) {
                    Some(problem) => {
                        self.problem(value.span(), format!("`{path}`: {problem}"));
                        None
                    }
                    None => Some(Value::Text(text.to_owned())),
                }
            }
            Form::List(_) => {
                let items = self.texts(&path, value, LIST_LIMIT)?;
                if items.iter().any(|item| item.is_empty() || item.len() > TEXT_LIMIT) {
                    self.problem(
                        value.span(),
                        format!("`{path}` has an empty entry or one of more than {TEXT_LIMIT} bytes"),
                    );
                    return None;
                }
                Some(Value::List(items))
            }
        }
    }

    /// A number inside its bounds. A value outside them takes the nearest bound.
    fn number(&mut self, path: &str, number: Number, value: &Spanned<DeValue<'_>>) -> Option<u64> {
        let Number { default, min, max, unit, period } = number;
        let Some(found) = integer(value) else {
            let range = format!("from {} to {}", amount(min, unit), amount(max, unit));
            self.problem(value.span(), format!("`{path}` is a whole number, {range}"));
            return None;
        };
        if found == 0 && period {
            let kept = amount(default, unit);
            self.problem(
                value.span(),
                format!(
                    "`{path}` = 0 is refused: a period or a time limit of 0 never ends its wait; the app keeps {kept}"
                ),
            );
            return None;
        }
        let clamped = found.clamp(i128::from(min), i128::from(max));
        let used = u64::try_from(clamped).unwrap_or(max);
        if clamped != found {
            let side = if found < i128::from(min) { "below its lowest value" } else { "above its highest value" };
            let bound = amount(used, unit);
            self.problem(value.span(), format!("`{path}` = {found} is {side}; the app uses {bound}"));
        }
        Some(used)
    }
}

/// "15 s", "70 %", "500": a number with its unit.
pub fn amount(value: u64, unit: Unit) -> String {
    match unit {
        Unit::Count => value.to_string(),
        Unit::Percent => format!("{value} %"),
        other => format!("{value} {}", other.text()),
    }
}

/// Why a text does not fit its setting, if it does not.
fn text_problem(id: Setting, text: &str) -> Option<String> {
    if text.len() > TEXT_LIMIT {
        return Some(format!("a text of {TEXT_LIMIT} bytes at most"));
    }
    match id {
        Setting::TimeZone if !is_time_zone(text) => {
            Some(format!("no time zone `{text}`; use local, UTC or a name such as Europe/Amsterdam"))
        }
        _ => None,
    }
}

/// True for "local", "UTC" and each name of the time zone database of the system.
pub fn is_time_zone(text: &str) -> bool {
    matches!(text, "local" | "UTC") || jiff::tz::TimeZone::get(text).is_ok()
}
