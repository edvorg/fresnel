//! The sections of a RoleBinding and of a ClusterRoleBinding (`research/01` section 3.4.4,
//! `research/02` section 4.6): the role as a link, and each subject with its kind, its name
//! and its namespace. A ServiceAccount is a link; a user and a group are names of the
//! authenticator of the cluster, with no object.
//!
//! `account_of` holds the rule of the API server for the namespace of a ServiceAccount, which
//! the view of a ServiceAccount uses to find its bindings.

use egui::Ui;
use fresnel_core::kinds::Kind;
use k8s_openapi::api::rbac::v1::{ClusterRoleBinding, RoleBinding, RoleRef, Subject};

use crate::views::detail::fields::{NONE, field, heading};
use crate::views::detail::lines::{details_heading, none, table};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;

pub(crate) fn role_binding(ui: &mut Ui, cx: &Context<'_>, binding: &RoleBinding) -> Option<Link> {
    show(ui, cx, &binding.role_ref, binding.subjects.as_deref().unwrap_or_default())
}

pub(crate) fn cluster_role_binding(ui: &mut Ui, cx: &Context<'_>, binding: &ClusterRoleBinding) -> Option<Link> {
    show(ui, cx, &binding.role_ref, binding.subjects.as_deref().unwrap_or_default())
}

fn show(ui: &mut Ui, cx: &Context<'_>, role: &RoleRef, subjects: &[Subject]) -> Option<Link> {
    let namespace = &cx.id.key.namespace;
    details_heading(ui, cx);
    let mut clicked = None;
    field(ui, cx.palette, "Role", |ui| clicked = role_link(ui, cx, role, namespace));
    ui.add_space(10.0);
    heading(ui, cx.palette, "SUBJECTS");
    if subjects.is_empty() {
        none(ui, cx);
    }
    table(ui, cx, "subjects", &["Kind", "Name", "Namespace"], subjects, |ui, subject| {
        ui.label(subject.kind.as_str());
        match account_of(subject, namespace) {
            Some(account) => {
                clicked = cx.link(ui, &subject.name, Kind::ServiceAccount, account, &subject.name).or(clicked.take())
            }
            None => {
                ui.label(subject.name.as_str());
            }
        }
        ui.label(account_of(subject, namespace).or(subject.namespace.as_deref()).unwrap_or(NONE));
    });
    ui.add_space(10.0);
    clicked
}

/// The role of a binding as a link: a Role of the namespace of the binding, or a ClusterRole.
pub(crate) fn role_link(ui: &mut Ui, cx: &Context<'_>, role: &RoleRef, namespace: &str) -> Option<Link> {
    let kind = if role.kind == "Role" { Kind::Role } else { Kind::ClusterRole };
    cx.link(ui, &format!("{} {}", role.kind, role.name), kind, namespace, &role.name)
}

/// The namespace of a ServiceAccount subject: its own, or else the namespace of its
/// RoleBinding, as the API server reads it. `None` for a user or a group, and for an account
/// with no namespace in a ClusterRoleBinding, which names no account.
pub(crate) fn account_of<'a>(subject: &'a Subject, binding_namespace: &'a str) -> Option<&'a str> {
    if subject.kind != "ServiceAccount" {
        return None;
    }
    let namespace = subject.namespace.as_deref().filter(|namespace| !namespace.is_empty()).unwrap_or(binding_namespace);
    (!namespace.is_empty()).then_some(namespace)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_service_account_takes_the_namespace_of_its_role_binding_and_a_user_has_none() {
        let subjects: Vec<Subject> = serde_json::from_value(json!([
            {"kind": "ServiceAccount", "name": "web", "namespace": "shop"},
            {"kind": "ServiceAccount", "name": "robot"},
            {"kind": "User", "name": "alice"},
        ]))
        .expect("subjects");
        let namespaces: Vec<Option<&str>> = subjects.iter().map(|subject| account_of(subject, "lab")).collect();
        assert_eq!(namespaces, [Some("shop"), Some("lab"), None]);
        assert_eq!(subjects.get(1).and_then(|subject| account_of(subject, "")), None, "a ClusterRoleBinding");
    }
}
