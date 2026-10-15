//! The lines and the tables of the sections of a kind, and the texts of the values that many
//! kinds share: a label selector, a count or a percent, a number of seconds, a long list.
//! Each kind writes such a value in one way, as kubectl does.

use egui::{Grid, RichText, Ui};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

use super::fields::{ITEM_LIMIT, NONE, heading, more, text_field};
use super::related::links_only;
use super::sections::Context;
use crate::format;

/// The heading of the details of a kind.
pub(crate) fn details_heading(ui: &mut Ui, cx: &Context<'_>) {
    heading(ui, cx.palette, "DETAILS");
}

/// A line with a value that the object can have, or the mark of no value.
pub(crate) fn line(ui: &mut Ui, cx: &Context<'_>, label: &str, value: Option<String>) {
    text_field(ui, cx.palette, label, value.as_deref().unwrap_or(NONE));
}

/// A table of a section: a line of titles, the rows that `row` draws for the first items,
/// and the count of the items that do not show. An object can have thousands of items, and
/// the panel draws each row in each frame. A section with no item has no table.
pub(crate) fn table<T>(
    ui: &mut Ui,
    cx: &Context<'_>,
    salt: &str,
    titles: &[&str],
    items: &[T],
    mut row: impl FnMut(&mut Ui, &T),
) {
    if items.is_empty() {
        return;
    }
    let grid = Grid::new((salt, &cx.id.cluster, &cx.id.key)).num_columns(titles.len()).spacing([12.0, 4.0]);
    grid.show(ui, |ui| {
        links_only(ui);
        for title in titles {
            ui.label(RichText::new(*title).size(11.0).color(cx.palette.text_faint));
        }
        ui.end_row();
        for item in items.iter().take(ITEM_LIMIT) {
            row(ui, item);
            ui.end_row();
        }
    });
    more(ui, cx.palette, items.len());
}

/// "None." under the heading of a section with no item.
pub(crate) fn none(ui: &mut Ui, cx: &Context<'_>) {
    ui.label(RichText::new("None.").color(cx.palette.text_faint));
}

/// The text of a label selector, as kubectl prints it: `app=web,tier in (a,b)`.
pub(crate) fn selector(selector: Option<&LabelSelector>) -> String {
    let Some(selector) = selector else {
        return NONE.to_owned();
    };
    let labels = selector.match_labels.iter().flatten().map(|(key, value)| format!("{key}={value}"));
    let expressions = selector.match_expressions.iter().flatten().map(|expression| {
        let values = expression.values.as_deref().unwrap_or_default().join(",");
        match expression.operator.as_str() {
            "In" => format!("{} in ({values})", expression.key),
            "NotIn" => format!("{} notin ({values})", expression.key),
            "Exists" => expression.key.clone(),
            "DoesNotExist" => format!("!{}", expression.key),
            other => format!("{} {other} ({values})", expression.key),
        }
    });
    let text: Vec<String> = labels.chain(expressions).collect();
    if text.is_empty() { "each object".to_owned() } else { text.join(",") }
}

/// True for a selector with a requirement: a selector with none selects each object.
pub(crate) fn narrows(selector: &LabelSelector) -> bool {
    selector.match_labels.as_ref().is_some_and(|labels| !labels.is_empty())
        || selector.match_expressions.as_ref().is_some_and(|expressions| !expressions.is_empty())
}

/// A count or a percent of a spec: `1` or `25%`.
pub(crate) fn number(value: &IntOrString) -> String {
    match value {
        IntOrString::Int(count) => count.to_string(),
        IntOrString::String(text) => text.clone(),
    }
}

/// A number of seconds, or the default of the server.
pub(crate) fn seconds(value: Option<i32>, default: Option<i32>) -> Option<String> {
    value.or(default).map(|seconds| format!("{seconds} s"))
}

/// The first items of a list as one text, and the count of the other ones: a rule of a Role
/// can name hundreds of resources.
pub(crate) fn list<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let mut items = items.into_iter();
    let shown: Vec<&str> = items.by_ref().take(ITEM_LIMIT).collect();
    let total = shown.len() + items.count();
    first_of(&shown, total)
}

/// The text of the first items of a list of `total` items, for a list that is too long to
/// make: the resources of a rule with each of its groups are a product of two lists.
pub(crate) fn first_of<T: AsRef<str>>(shown: &[T], total: usize) -> String {
    let text = shown.iter().map(AsRef::as_ref).collect::<Vec<_>>().join(", ");
    match total.saturating_sub(shown.len()) {
        0 => text,
        more => format!("{text} and {} more", format::thousands(more as u64)),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_selector_reads_as_the_text_of_kubectl() {
        let selector: LabelSelector = serde_json::from_value(json!({"matchLabels": {"app": "web"},
            "matchExpressions": [{"key": "tier", "operator": "In", "values": ["a", "b"]},
                {"key": "canary", "operator": "DoesNotExist"}]}))
        .expect("a selector");
        assert_eq!(super::selector(Some(&selector)), "app=web,tier in (a,b),!canary");
        assert_eq!(super::selector(Some(&LabelSelector::default())), "each object");
        assert_eq!(super::selector(None), NONE);
    }

    #[test]
    fn a_long_list_shows_its_first_items_and_the_count_of_the_others() {
        assert_eq!(list(["get", "list"]), "get, list");
        let names: Vec<String> = (0..ITEM_LIMIT + 3).map(|number| number.to_string()).collect();
        let text = list(names.iter().map(String::as_str));
        assert!(text.starts_with("0, 1, 2") && text.ends_with(&format!("{} and 3 more", ITEM_LIMIT - 1)), "{text}");
    }
}
