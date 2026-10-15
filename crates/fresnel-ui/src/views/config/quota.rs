//! The sections of a ResourceQuota and of a LimitRange (`research/01` section 3.4.5,
//! `research/02` section 4.3): the use of each resource against its hard limit with a bar,
//! and the limits of each type of object. The view of a Namespace shows the same tables for
//! each quota and each limit range of the namespace.
//!
//! The texts of a quantity are the texts of the server. The bar compares their numbers
//! (`fresnel_core::quantity`), and a quantity that is not a number has no bar.

use std::collections::{BTreeMap, BTreeSet};

use egui::{RichText, Ui};
use fresnel_core::quantity;
use k8s_openapi::api::core::v1::{LimitRange, LimitRangeItem, ResourceQuota};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

use crate::views::detail::fields::{NONE, heading};
use crate::views::detail::lines::{details_heading, line, none, table};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;
use crate::widgets::bar;

/// The part of a limit from which the bar warns.
const NEAR: f64 = 0.8;

pub(crate) fn quota_details(ui: &mut Ui, cx: &Context<'_>, quota: &ResourceQuota) -> Option<Link> {
    let spec = quota.spec.as_ref();
    details_heading(ui, cx);
    let scopes = spec.and_then(|spec| spec.scopes.as_deref()).filter(|scopes| !scopes.is_empty());
    line(ui, cx, "Scopes", Some(scopes.map_or_else(|| "each object".to_owned(), |scopes| scopes.join(", "))));
    let selector =
        spec.and_then(|spec| spec.scope_selector.as_ref()).and_then(|selector| selector.match_expressions.as_ref());
    if let Some(expressions) = selector.filter(|expressions| !expressions.is_empty()) {
        let text = expressions.iter().map(|expression| {
            let values = expression.values.as_deref().unwrap_or_default().join(",");
            format!("{} {} ({values})", expression.scope_name, expression.operator)
        });
        line(ui, cx, "Scope selector", Some(text.collect::<Vec<_>>().join(", ")));
    }
    ui.add_space(10.0);
    heading(ui, cx.palette, "USE");
    usage(ui, cx, quota);
    ui.add_space(10.0);
    None
}

/// The use of each resource of a quota against its hard limit. The limit of the status is
/// the one that the server applies; a new quota has only the limit of its spec.
pub(crate) fn usage(ui: &mut Ui, cx: &Context<'_>, quota: &ResourceQuota) {
    let status = quota.status.as_ref();
    let hard =
        status.and_then(|status| status.hard.as_ref()).or(quota.spec.as_ref().and_then(|spec| spec.hard.as_ref()));
    let used = status.and_then(|status| status.used.as_ref());
    let rows: Vec<(&String, &Quantity)> = hard.into_iter().flatten().collect();
    if rows.is_empty() {
        none(ui, cx);
    }
    let salt = format!("use of {}", quota.metadata.name.as_deref().unwrap_or_default());
    table(ui, cx, &salt, &["Resource", "Used", "Hard", "Use"], &rows, |ui, (resource, hard)| {
        let now = used.and_then(|used| used.get(*resource));
        ui.label(resource.as_str());
        ui.label(now.map_or(NONE, |now| now.0.as_str()));
        ui.label(hard.0.as_str());
        match now.and_then(|now| part(now, hard)) {
            Some(part) => {
                let fill = match part {
                    part if part >= 1.0 => cx.palette.error,
                    part if part >= NEAR => cx.palette.warn,
                    _ => cx.palette.ok,
                };
                ui.horizontal(|ui| {
                    bar(ui, part, fill, cx.palette.bg_raised);
                    ui.label(RichText::new(format!("{:.0}%", part * 100.0)).color(cx.palette.text_muted));
                });
            }
            None => {
                ui.label(RichText::new(NONE).color(cx.palette.text_faint));
            }
        }
    });
}

/// The part of `hard` that `used` takes. `None` for a quantity that is not a number, and for
/// a limit of 0, which allows nothing.
fn part(used: &Quantity, hard: &Quantity) -> Option<f64> {
    let hard = quantity::number(&hard.0).filter(|hard| *hard > 0.0)?;
    Some(quantity::number(&used.0)? / hard)
}

pub(crate) fn range_details(ui: &mut Ui, cx: &Context<'_>, range: &LimitRange) -> Option<Link> {
    heading(ui, cx.palette, "LIMITS");
    limits(ui, cx, range);
    ui.add_space(10.0);
    None
}

/// One row for each resource of each type: Container, Pod or PersistentVolumeClaim.
pub(crate) fn limits(ui: &mut Ui, cx: &Context<'_>, range: &LimitRange) {
    let items = range.spec.as_ref().map(|spec| spec.limits.as_slice()).unwrap_or_default();
    let rows: Vec<(&LimitRangeItem, &String)> =
        items.iter().flat_map(|item| resources(item).into_iter().map(move |name| (item, name))).collect();
    if rows.is_empty() {
        none(ui, cx);
    }
    let salt = format!("limits of {}", range.metadata.name.as_deref().unwrap_or_default());
    let titles = ["Type", "Resource", "Min", "Max", "Default request", "Default limit", "Max ratio"];
    table(ui, cx, &salt, &titles, &rows, |ui, (item, name)| {
        ui.label(item.type_.as_str());
        ui.label(name.as_str());
        for map in [&item.min, &item.max, &item.default_request, &item.default, &item.max_limit_request_ratio] {
            let value = map.as_ref().and_then(|map| map.get(*name));
            ui.label(value.map_or(NONE, |value| value.0.as_str()));
        }
    });
}

/// The resources that an item of a LimitRange names, in the sequence of their names.
fn resources(item: &LimitRangeItem) -> BTreeSet<&String> {
    let maps = [&item.min, &item.max, &item.default_request, &item.default, &item.max_limit_request_ratio];
    maps.into_iter().flatten().flat_map(BTreeMap::keys).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_part_of_a_limit_compares_the_numbers_of_two_quantities() {
        let part = |used: &str, hard: &str| part(&Quantity(used.to_owned()), &Quantity(hard.to_owned()));
        assert_eq!(part("6", "20"), Some(0.3));
        assert_eq!(part("500m", "2"), Some(0.25));
        assert_eq!(part("4Gi", "8Gi"), Some(0.5));
        assert_eq!(part("1", "0"), None, "a limit of 0 allows nothing");
        assert_eq!(part("many", "20"), None);
    }
}
