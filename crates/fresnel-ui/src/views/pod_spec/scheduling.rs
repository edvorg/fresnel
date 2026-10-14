//! The lines of a spec that tell where and as whom its pods run: the ServiceAccount, the
//! node selector, the tolerations, the priority class and the runtime class.

use egui::{Label, RichText, Ui};
use fresnel_core::kinds::Kind;
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::core::v1::PodSpec;

use super::{Place, texts};
use crate::format::short;
use crate::views::detail::fields::{ITEM_LIMIT, NONE, VALUE_LIMIT, chips, field, more};
use crate::views::objects::Link;

/// Draws the lines. Gives the object of a link that the user clicked.
pub(crate) fn show(ui: &mut Ui, place: &Place<'_>, spec: &PodSpec) -> Option<Link> {
    let palette = place.palette;
    let mut clicked = None;
    let account = spec.service_account_name.as_deref().filter(|name| !name.is_empty()).unwrap_or("default");
    field(ui, palette, "Service account", |ui| {
        clicked = place.link(ui, account, Kind::ServiceAccount, account);
    });
    field(ui, palette, "Node selector", |ui| chips(ui, palette, spec.node_selector.as_ref()));
    let tolerations = spec.tolerations.as_deref().unwrap_or_default();
    field(ui, palette, "Tolerations", |ui| {
        if tolerations.is_empty() {
            ui.label(RichText::new(NONE).color(palette.text_faint));
        }
        for toleration in tolerations.iter().take(ITEM_LIMIT) {
            ui.add(Label::new(short(&texts::toleration(toleration), VALUE_LIMIT)).wrap());
        }
        more(ui, palette, tolerations.len());
    });
    // A priority class and a runtime class are objects of the cluster, with no namespace.
    let of_cluster = |ui: &mut Ui, label: &str, kind: Kind, name: Option<&str>| {
        let name = name.filter(|name| !name.is_empty())?;
        let mut link = None;
        field(ui, palette, label, |ui| {
            if ui.link(short(name, VALUE_LIMIT)).clicked() {
                link = Some(Link { cluster: place.cluster.clone(), kind, key: ObjectKey::new("", name) });
            }
        });
        link
    };
    let priority = of_cluster(ui, "Priority class", Kind::PriorityClass, spec.priority_class_name.as_deref());
    let runtime = of_cluster(ui, "Runtime class", Kind::RuntimeClass, spec.runtime_class_name.as_deref());
    clicked.or(priority).or(runtime)
}
