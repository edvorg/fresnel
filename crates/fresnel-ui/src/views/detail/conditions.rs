//! The conditions of an object (plan section 11.2): the list in its status. Each kind has
//! a type of its own for them, so the panel reads them from the JSON of the object, one
//! time for each new version of it. A condition can mean good news with `True`, as `Ready`,
//! or bad news, as `MemoryPressure`, so the section gives the status no color.

use std::time::SystemTime;

use egui::{Align, Label, Layout, RichText, Ui};
use jiff::Timestamp;
use serde::Serialize;
use serde_json::Value;

use super::fields::{ITEM_LIMIT, NONE, heading, more};
use crate::format::{self, Clock, short};
use crate::theme::{Palette, SMALL};

/// The most characters of the type, the status and the reason of one condition.
const NAME_LIMIT: usize = 80;
/// The most characters of the message of one condition.
const MESSAGE_LIMIT: usize = 2000;

/// One condition, as the section shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Condition {
    kind: String,
    status: String,
    reason: String,
    message: String,
    changed: Option<SystemTime>,
}

/// The conditions in the status of an object, in their sequence, and their count. An
/// object with no list has none.
pub(super) fn of<K: Serialize>(object: &K) -> (Vec<Condition>, usize) {
    let Ok(value) = serde_json::to_value(object) else {
        return (Vec::new(), 0);
    };
    let Some(list) = value.pointer("/status/conditions").and_then(Value::as_array) else {
        return (Vec::new(), 0);
    };
    let text = |condition: &Value, field: &str, limit| {
        short(condition.get(field).and_then(Value::as_str).unwrap_or_default(), limit).into_owned()
    };
    let read = |condition: &Value| Condition {
        kind: text(condition, "type", NAME_LIMIT),
        status: text(condition, "status", NAME_LIMIT),
        reason: text(condition, "reason", NAME_LIMIT),
        message: text(condition, "message", MESSAGE_LIMIT),
        changed: condition
            .get("lastTransitionTime")
            .and_then(Value::as_str)
            .and_then(|time| time.parse::<Timestamp>().ok())
            .map(SystemTime::from),
    };
    (list.iter().take(ITEM_LIMIT).map(read).collect(), list.len())
}

/// Draws the section. An object with no condition has no section.
pub(super) fn show(ui: &mut Ui, palette: &Palette, clock: &Clock, conditions: &[Condition], total: usize) {
    if conditions.is_empty() {
        return;
    }
    heading(ui, palette, "CONDITIONS");
    for condition in conditions {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let kind = if condition.kind.is_empty() { NONE } else { &condition.kind };
            ui.label(RichText::new(kind).strong());
            ui.label(RichText::new(&condition.status).color(palette.text));
            if !condition.reason.is_empty() {
                ui.label(RichText::new(&condition.reason).size(SMALL).color(palette.text_muted));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let changed = condition.changed.map(|time| format!("{} ago", format::span(clock.since(time))));
                let changed = changed.unwrap_or_else(|| NONE.to_owned());
                ui.label(RichText::new(changed).size(SMALL).color(palette.text_faint));
            });
        });
        if !condition.message.is_empty() {
            ui.add(Label::new(RichText::new(&condition.message).color(palette.text_muted)).wrap());
        }
        ui.add_space(4.0);
    }
    more(ui, palette, total);
    ui.add_space(10.0);
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::apps::v1::Deployment;
    use k8s_openapi::api::core::v1::ConfigMap;

    use super::*;

    #[test]
    fn the_conditions_come_from_the_json_of_each_kind_and_a_kind_with_none_has_none() {
        let text = r#"{"metadata":{"name":"web"},"status":{"conditions":[
            {"type":"Available","status":"True","reason":"MinimumReplicasAvailable","message":"ok",
             "lastTransitionTime":"2027-01-15T07:00:00Z"},
            {"type":"Progressing","status":"False"}]}}"#;
        let deployment: Deployment = serde_json::from_str(text).expect("a Deployment");
        let (conditions, total) = of(&deployment);
        assert_eq!(total, 2);
        let available = &conditions[0];
        assert_eq!((available.kind.as_str(), available.status.as_str()), ("Available", "True"));
        assert_eq!((available.reason.as_str(), available.message.as_str()), ("MinimumReplicasAvailable", "ok"));
        assert_eq!(available.changed, Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_799_996_400)));
        assert_eq!((conditions[1].reason.as_str(), conditions[1].changed), ("", None));
        assert_eq!(of(&ConfigMap::default()), (Vec::new(), 0));
    }
}
