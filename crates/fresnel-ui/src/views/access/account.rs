//! The sections of a ServiceAccount (`research/01` section 3.4.4, `research/02` section 4.6):
//! the mount of its token, its Secrets and its image pull Secrets as links; then the role
//! bindings that name it, with their role, from the stores of the full RoleBindings and
//! ClusterRoleBindings that the page of the ServiceAccounts holds (`Command::ShowKinds`).
//!
//! The pods that run as the account come from the reference index ("USED BY").

use egui::Ui;
use fresnel_core::app::ClusterData;
use fresnel_core::kinds::{Describe, Kind};
use fresnel_core::published::Held;
use fresnel_core::store::Snapshot;
use k8s_openapi::api::core::v1::ServiceAccount;
use k8s_openapi::api::rbac::v1::{ClusterRoleBinding, RoleBinding, RoleRef, Subject};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::bindings::{account_of, role_link};
use crate::views::detail::fields::{ITEM_LIMIT, NONE, field, heading, more, note};
use crate::views::detail::lines::{details_heading, line, none, table};
use crate::views::detail::related::links_only;
use crate::views::detail::sections::Context;
use crate::views::objects::{Link, old_since};

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, account: &ServiceAccount) -> Option<Link> {
    let namespace = &cx.id.key.namespace;
    details_heading(ui, cx);
    let mount = match account.automount_service_account_token {
        Some(false) => "no: a pod mounts the token only when its spec asks",
        _ => "yes",
    };
    line(ui, cx, "Mount token", Some(mount.to_owned()));
    let mut clicked = None;
    let secrets: Vec<&str> = account.secrets.iter().flatten().filter_map(|secret| secret.name.as_deref()).collect();
    let pulls: Vec<&str> = account.image_pull_secrets.iter().flatten().map(|secret| secret.name.as_str()).collect();
    for (label, names) in [("Secrets", secrets), ("Image pull", pulls)] {
        field(ui, cx.palette, label, |ui| {
            if names.is_empty() {
                ui.label(NONE);
                return;
            }
            ui.horizontal_wrapped(|ui| {
                links_only(ui);
                for name in names.iter().take(ITEM_LIMIT) {
                    clicked = cx.link(ui, name, Kind::Secret, namespace, name).or(clicked.take());
                }
            });
            more(ui, cx.palette, names.len());
        });
    }
    ui.add_space(10.0);
    clicked
}

/// The bindings that name the account.
pub(crate) fn parts(ui: &mut Ui, cx: &Context<'_>) -> Option<Link> {
    heading(ui, cx.palette, "ROLE BINDINGS");
    let Some(data) = cx.data else {
        note(ui, cx.palette.warn, "The bindings are not loaded: the cluster is not connected.");
        ui.add_space(10.0);
        return None;
    };
    let (Some(roles), Some(clusters)) = (held::<RoleBinding>(ui, cx, data), held::<ClusterRoleBinding>(ui, cx, data))
    else {
        ui.add_space(10.0);
        return None;
    };
    let names = |subjects: Option<&Vec<Subject>>, namespace: &str| {
        subjects.into_iter().flatten().any(|subject| {
            subject.name == *cx.id.key.name && account_of(subject, namespace) == Some(&*cx.id.key.namespace)
        })
    };
    let mut rows: Vec<(Kind, &str, &str, &RoleRef)> = Vec::new();
    for binding in roles.items.iter().map(|(_, binding)| binding) {
        let namespace = binding.metadata.namespace.as_deref().unwrap_or_default();
        if names(binding.subjects.as_ref(), namespace) {
            rows.push((Kind::RoleBinding, namespace, name(&binding.metadata), &binding.role_ref));
        }
    }
    for binding in clusters.items.iter().map(|(_, binding)| binding) {
        if names(binding.subjects.as_ref(), "") {
            rows.push((Kind::ClusterRoleBinding, "", name(&binding.metadata), &binding.role_ref));
        }
    }
    if rows.is_empty() && roles.listed && clusters.listed {
        none(ui, cx);
    }
    let mut clicked = None;
    table(ui, cx, "bindings", &["Binding", "Role"], &rows, |ui, (kind, namespace, binding, role)| {
        let text = format!("{} {binding}", kind.info().name);
        clicked = cx.link(ui, &text, *kind, namespace, binding).or(clicked.take());
        clicked = role_link(ui, cx, role, namespace).or(clicked.take());
    });
    ui.add_space(10.0);
    clicked
}

/// The snapshot of the full bindings of a kind, with a note when it is not current.
fn held<K: Describe>(ui: &mut Ui, cx: &Context<'_>, data: &ClusterData) -> Option<Held<Snapshot<K>>> {
    let Some(snapshot) = data.snapshot::<K>() else {
        note(ui, cx.palette.warn, &format!("The {} are not loaded: their store starts.", K::TITLE));
        return None;
    };
    if !snapshot.listed {
        note(ui, cx.palette.warn, &format!("The {} are not loaded: their store {}.", K::TITLE, snapshot.state));
    } else if let Some(old) = old_since(&snapshot, cx.clock) {
        note(ui, cx.palette.warn, &format!("The {} are {old}.", K::TITLE));
    }
    Some(snapshot)
}

fn name(meta: &ObjectMeta) -> &str {
    meta.name.as_deref().unwrap_or_default()
}
