//! The filters of the quick search (plan section 12.1): the clusters and the kinds that it
//! reads. Each one is a menu with a switch for each choice; no choice is each one.

use std::collections::BTreeSet;

use egui::containers::menu::{MenuButton, MenuConfig};
use egui::{PopupCloseBehavior, RichText, ScrollArea, Ui};
use fresnel_core::app::CoreView;
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::{Kind, Sidebar};
use fresnel_core::search::Query;

use crate::theme::{self, SMALL};

/// One choice of a menu: its value, its words and the heading of its group before it.
type Item<T> = (T, String, Option<&'static str>);

/// Draws the two menus. Gives true when the user changed a filter.
pub(super) fn show(ui: &mut Ui, query: &mut Query, view: &CoreView) -> bool {
    let selected = view.clusters.iter().filter(|row| row.selected).map(|row| row.id.clone());
    // A cluster of the filter that left the selection stays in the menu, so the user can
    // remove it from the filter.
    let clusters: BTreeSet<ClusterId> = selected.chain(query.clusters.iter().cloned()).collect();
    let clusters: Vec<Item<ClusterId>> =
        clusters.into_iter().map(|cluster| (cluster.clone(), cluster.as_str().to_owned(), None)).collect();
    let mut changed = menu(ui, &mut query.clusters, &clusters, ("All clusters", "Cluster", "clusters"));
    // The kinds come in the groups of the sidebar.
    let kinds: Vec<Item<Kind>> = Sidebar::ALL
        .iter()
        .flat_map(|group| group.kinds().enumerate().map(|(place, kind)| (*group, place, kind)))
        .map(|(group, place, kind)| (kind, kind.info().title.to_owned(), (place == 0).then(|| group.title())))
        .collect();
    changed |= menu(ui, &mut query.kinds, &kinds, ("All kinds", "Kind", "kinds"));
    changed
}

/// A menu with a switch for each item. `titles` are the title of the menu with no choice,
/// the noun of one choice and the noun of more of them.
fn menu<T: Ord + Clone>(ui: &mut Ui, chosen: &mut BTreeSet<T>, items: &[Item<T>], titles: (&str, &str, &str)) -> bool {
    let (none, one, many) = titles;
    let words_of = |value: &T| items.iter().find(|(item, ..)| item == value).map_or("?", |(_, words, _)| words);
    let title = match chosen.len() {
        0 => none.to_owned(),
        1 => format!("{one}: {}", chosen.first().map_or("?", words_of)),
        count => format!("{count} {many}"),
    };
    let before = chosen.clone();
    let config = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
    MenuButton::new(title).config(config).ui(ui, |ui| {
        if ui.button(none).clicked() {
            chosen.clear();
        }
        ui.separator();
        ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for (value, words, heading) in items {
                if let Some(heading) = heading {
                    let color = theme::palette(ui).text_faint;
                    ui.label(RichText::new(heading.to_uppercase()).size(SMALL).color(color));
                }
                let mut on = chosen.contains(value);
                if ui.checkbox(&mut on, words).changed() {
                    if on {
                        chosen.insert(value.clone());
                    } else {
                        chosen.remove(value);
                    }
                }
            }
        });
    });
    *chosen != before
}
