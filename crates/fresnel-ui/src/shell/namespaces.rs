//! The namespace filter of the top bar (plan section 11.1): one filter for each view, so a
//! page of an other kind shows the same namespaces. A kind with no namespace ignores it.
//!
//! The menu lists the namespaces of the Namespace store of each selected cluster, the
//! namespaces of the objects on the page, and each namespace of the filter: a namespace
//! whose last object went away stays in the menu, so the user can remove it from the filter.
//! The favorite namespaces of the selected clusters in the settings come first.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::containers::menu::{MenuButton, MenuConfig};
use egui::{PopupCloseBehavior, ScrollArea, Ui};
use fresnel_core::app::CoreView;
use fresnel_core::kinds::Kind;
use k8s_openapi::api::core::v1::Namespace;

/// Draws the menu. `on_page` are the namespaces of the objects of the open page.
pub(super) fn menu(ui: &mut Ui, view: &CoreView, on_page: &[Arc<str>], chosen: &mut BTreeSet<Arc<str>>) {
    let title = match chosen.len() {
        0 => "All namespaces".to_owned(),
        1 => chosen.first().map_or_else(String::new, |only| format!("Namespace: {only}")),
        more => format!("{more} namespaces"),
    };
    // The menu stays open at a click: the user selects more than one namespace.
    let config = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
    MenuButton::new(title).config(config).ui(ui, |ui| {
        if ui.button("All namespaces").clicked() {
            chosen.clear();
        }
        ui.separator();
        let favorites: BTreeSet<Arc<str>> = favorites(view).collect();
        let listed: BTreeSet<Arc<str>> =
            of_clusters(view).chain(on_page.iter().cloned()).chain(chosen.iter().cloned()).collect();
        ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for namespace in favorites.iter().chain(listed.difference(&favorites)) {
                let mut on = chosen.contains(namespace);
                let text =
                    if favorites.contains(namespace) { format!("★ {namespace}") } else { namespace.to_string() };
                if ui.checkbox(&mut on, text).changed() {
                    if on {
                        chosen.insert(Arc::clone(namespace));
                    } else {
                        chosen.remove(namespace);
                    }
                }
            }
        });
    });
}

/// The favorite namespaces of the selected clusters in the settings.
fn favorites(view: &CoreView) -> impl Iterator<Item = Arc<str>> + '_ {
    let selected = view.clusters.iter().filter(|row| row.selected);
    let options = selected.filter_map(|row| view.config.options(&row.id));
    options.flat_map(|options| options.favorite_namespaces.iter().map(|name| Arc::from(name.as_str())))
}

/// The namespaces of the Namespace stores of the selected clusters.
fn of_clusters(view: &CoreView) -> impl Iterator<Item = Arc<str>> + '_ {
    let data = view.clusters.iter().filter(|row| row.selected).filter_map(|row| row.data.as_ref());
    let stores = data.filter_map(|data| data.sources.iter().find(|source| source.status.kind == Kind::Namespace));
    stores
        .filter_map(|source| source.snapshot::<Namespace>())
        .flat_map(|snapshot| snapshot.items.iter().map(|(key, _)| Arc::clone(&key.name)).collect::<Vec<_>>())
}
