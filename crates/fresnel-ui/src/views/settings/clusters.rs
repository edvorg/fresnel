//! The options of each cluster of the kubeconfig files, and the cluster groups.

use egui::{ComboBox, Grid, RichText, TextEdit, Ui};
use fresnel_core::app::CoreView;
use fresnel_core::cluster::ClusterId;
use fresnel_core::config::{ALIAS_LIMIT, ClusterOptions, Config, ConfirmRule, EditValue, Rgb, is_namespace};

use super::draft::{Draft, Path, lines, path};
use crate::theme::{Palette, SMALL};
use crate::widgets::{self, dot};

/// The choice of an option that `[safety]` gives each cluster: the text of "no option".
const AS_SAFETY: &str = "as [safety]";

/// Draws the options of each cluster.
pub(super) fn options(ui: &mut Ui, palette: &Palette, view: &CoreView, draft: &mut Draft) {
    if view.clusters.is_empty() {
        ui.label(RichText::new("The kubeconfig files have no context.").color(palette.text_muted));
    }
    for row in &view.clusters {
        let name = row.id.as_str();
        let own = view.config.options(&row.id).cloned().unwrap_or_default();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let color =
                view.config.color(&row.id).map_or(palette.accent, |Rgb([r, g, b])| egui::Color32::from_rgb(r, g, b));
            dot(ui, color);
            ui.label(RichText::new(name).strong());
        });
        Grid::new(("cluster options", name)).num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
            texts(ui, palette, draft, &row.id, &own);
            choices(ui, &view.config, draft, &row.id, &own);
        });
    }
}

fn texts(ui: &mut Ui, palette: &Palette, draft: &mut Draft, id: &ClusterId, own: &ClusterOptions) {
    let key = |option: &str| path(&["clusters", id.as_str(), option]);
    let alias = own.alias.clone().unwrap_or_default();
    row(ui, "Alias", |ui| {
        draft.field(ui, palette, (&key("alias"), &format!("Alias of {id}"), false), &alias, |text| match text.trim() {
            "" => Ok(EditValue::Remove),
            alias if alias.chars().count() <= ALIAS_LIMIT => Ok(EditValue::Text(alias.to_owned())),
            _ => Err(format!("{ALIAS_LIMIT} characters at most")),
        });
    });
    let color = own.color.map(|rgb| rgb.to_string()).unwrap_or_default();
    row(ui, "Color", |ui| {
        draft.field(ui, palette, (&key("color"), &format!("Color of {id}"), false), &color, |text| match text.trim() {
            "" => Ok(EditValue::Remove),
            color if Rgb::parse(color).is_some() => Ok(EditValue::Text(color.to_owned())),
            _ => Err("a color such as #3fb950".to_owned()),
        });
    });
    let namespace = own.default_namespace.clone().unwrap_or_default();
    row(ui, "Default namespace", |ui| {
        let name = format!("Default namespace of {id}");
        draft.field(ui, palette, (&key("default_namespace"), &name, false), &namespace, |text| match text.trim() {
            "" => Ok(EditValue::Remove),
            namespace if is_namespace(namespace) => Ok(EditValue::Text(namespace.to_owned())),
            namespace => Err(no_namespace(namespace)),
        });
    });
    let favorites = own.favorite_namespaces.join("\n");
    row(ui, "Favorite namespaces", |ui| {
        let name = format!("Favorite namespaces of {id}");
        draft.field(ui, palette, (&key("favorite_namespaces"), &name, true), &favorites, |text| {
            let names = lines(text);
            match names.iter().find(|name| !is_namespace(name)) {
                Some(name) => Err(no_namespace(name)),
                None => Ok(EditValue::List(names)),
            }
        });
    });
}

/// The reason for a text that is no name of a namespace, which the reader of the file
/// would drop.
fn no_namespace(text: &str) -> String {
    format!("`{text}` is no name of a namespace: small letters, digits and -")
}

fn choices(ui: &mut Ui, config: &Config, draft: &mut Draft, id: &ClusterId, own: &ClusterOptions) {
    let key = |option: &str| path(&["clusters", id.as_str(), option]);
    let read_only = match own.read_only {
        Some(true) => "read-only",
        Some(false) => "writable",
        None => AS_SAFETY,
    };
    let now = if config.read_only(id) { "read-only now" } else { "writable now" };
    row(ui, "Changes", |ui| {
        let picked = choice(
            ui,
            &key("read_only"),
            &format!("Changes of {id}"),
            read_only,
            &[AS_SAFETY, "read-only", "writable"],
        );
        let value = |text: &str| match text {
            "read-only" => EditValue::Flag(true),
            "writable" => EditValue::Flag(false),
            _ => EditValue::Remove,
        };
        set(draft, &key("read_only"), picked.map(|text| value(&text)), value(read_only));
        ui.label(RichText::new(now).size(SMALL));
    });
    let confirm = match own.confirm {
        Some(ConfirmRule::Dialog) => "dialog",
        Some(ConfirmRule::TypeName) => "type-name",
        None => AS_SAFETY,
    };
    row(ui, "Confirm rule", |ui| {
        let options = [AS_SAFETY, "dialog", "type-name"];
        let picked = choice(ui, &key("confirm"), &format!("Confirm rule of {id}"), confirm, &options);
        let value = |text: &str| match text {
            AS_SAFETY => EditValue::Remove,
            rule => EditValue::Text(rule.to_owned()),
        };
        set(draft, &key("confirm"), picked.map(|text| value(&text)), value(confirm));
    });
}

/// A combo box of texts. Gives the text that the user picked in this frame.
fn choice(ui: &mut Ui, key: &Path, name: &str, shown: &str, options: &[&str]) -> Option<String> {
    let mut picked = shown.to_owned();
    let combo = ComboBox::from_id_salt(key).selected_text(shown).show_ui(ui, |ui| {
        for option in options {
            ui.selectable_value(&mut picked, (*option).to_owned(), *option);
        }
    });
    widgets::name(&combo.response, name);
    (picked != shown).then_some(picked)
}

fn set(draft: &mut Draft, key: &Path, picked: Option<EditValue>, now: EditValue) {
    if let Some(value) = picked {
        draft.set(key, value, &now);
    }
}

fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.label(label);
    ui.horizontal(add);
    ui.end_row();
}

/// Draws the cluster groups, each with its clusters, and a new group of the selected clusters.
pub(super) fn groups(ui: &mut Ui, palette: &Palette, view: &CoreView, draft: &mut Draft, new: &mut String) {
    let note = "A group selects its clusters with one click in the top bar.";
    ui.label(RichText::new(note).size(SMALL).color(palette.text_muted));
    Grid::new("cluster groups").num_columns(3).spacing([16.0, 4.0]).show(ui, |ui| {
        for group in &view.config.groups {
            let key = path(&["groups", group.name.as_str()]);
            ui.label(&group.name);
            let name = format!("Clusters of the group {}", group.name);
            draft.field(ui, palette, (&key, &name, true), &group.clusters.join("\n"), |text| {
                Ok(EditValue::List(lines(text)))
            });
            draft.reset(ui, (&key, &group.name), true);
            ui.end_row();
        }
    });
    ui.horizontal(|ui| {
        let field = ui.add(TextEdit::singleline(new).hint_text("Name of a new group").desired_width(200.0));
        widgets::name(&field, "Name of a new group");
        let selected: Vec<String> =
            view.clusters.iter().filter(|row| row.selected).map(|row| row.id.to_string()).collect();
        let fits = !new.trim().is_empty() && !selected.is_empty();
        if ui.add_enabled(fits, egui::Button::new("Add a group of the selected clusters")).clicked() {
            let key = path(&["groups", new.trim()]);
            draft.set(&key, EditValue::List(selected), &EditValue::Remove);
            new.clear();
        }
    });
}
