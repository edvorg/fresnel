//! The rules of a Role and of a ClusterRole, as `kubectl describe` prints them: one row for
//! each rule with its resources (`deployments.apps`), its URLs that are not resources, the
//! names that it is for, and its verbs. A ClusterRole that aggregates other ones names
//! their labels.

use egui::Ui;
use k8s_openapi::api::rbac::v1::{ClusterRole, PolicyRule, Role};

use crate::views::detail::fields::{NONE, heading};
use crate::views::detail::lines::{line, list, none, selector, table};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;

pub(crate) fn role(ui: &mut Ui, cx: &Context<'_>, role: &Role) -> Option<Link> {
    show(ui, cx, role.rules.as_deref().unwrap_or_default());
    None
}

pub(crate) fn cluster_role(ui: &mut Ui, cx: &Context<'_>, role: &ClusterRole) -> Option<Link> {
    let selectors = role.aggregation_rule.as_ref().and_then(|rule| rule.cluster_role_selectors.as_deref());
    if let Some(selectors) = selectors {
        heading(ui, cx.palette, "AGGREGATION");
        let labels: Vec<String> = selectors.iter().map(|labels| selector(Some(labels))).collect();
        line(ui, cx, "ClusterRoles", Some(format!("each one with {}", labels.join(" or "))));
        ui.add_space(10.0);
    }
    show(ui, cx, role.rules.as_deref().unwrap_or_default());
    None
}

fn show(ui: &mut Ui, cx: &Context<'_>, rules: &[PolicyRule]) {
    heading(ui, cx.palette, "RULES");
    if rules.is_empty() {
        none(ui, cx);
    }
    let rows: Vec<[String; 4]> = rules.iter().map(row).collect();
    table(ui, cx, "rules", &["Resources", "URLs", "Names", "Verbs"], &rows, |ui, row| {
        for text in row {
            ui.label(if text.is_empty() { NONE } else { text.as_str() });
        }
    });
    ui.add_space(10.0);
}

/// The resources with their group, the URLs, the names and the verbs of one rule.
fn row(rule: &PolicyRule) -> [String; 4] {
    let groups = rule.api_groups.as_deref().unwrap_or_default();
    let resources = rule.resources.iter().flatten().flat_map(|resource| {
        groups.iter().map(move |group| if group.is_empty() { resource.clone() } else { format!("{resource}.{group}") })
    });
    let resources: Vec<String> = resources.collect();
    let words = |items: Option<&Vec<String>>| list(items.into_iter().flatten().map(String::as_str));
    [
        list(resources.iter().map(String::as_str)),
        words(rule.non_resource_urls.as_ref()),
        words(rule.resource_names.as_ref()),
        words(Some(&rule.verbs)),
    ]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_rule_reads_as_kubectl_describe_prints_it() {
        let rules: Vec<PolicyRule> = serde_json::from_value(json!([
            {"apiGroups": ["", "apps"], "resources": ["pods", "deployments"], "verbs": ["get", "list"]},
            {"apiGroups": ["*"], "resources": ["*"], "resourceNames": ["web"], "verbs": ["*"]},
            {"nonResourceURLs": ["/healthz", "/metrics"], "verbs": ["get"]},
        ]))
        .expect("rules");
        let rows: Vec<[String; 4]> = rules.iter().map(row).collect();
        assert_eq!(rows[0], ["pods, pods.apps, deployments, deployments.apps", "", "", "get, list"].map(str::to_owned));
        assert_eq!(rows[1], ["*.*", "", "web", "*"].map(str::to_owned));
        assert_eq!(rows[2], ["", "/healthz, /metrics", "", "get"].map(str::to_owned));
    }
}
