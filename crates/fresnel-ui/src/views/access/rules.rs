//! The rules of a Role and of a ClusterRole, as `kubectl describe` prints them: one row for
//! each rule with its resources (`deployments.apps`), its URLs that are not resources, the
//! names that it is for, and its verbs. A ClusterRole that aggregates other ones names
//! their labels.

use egui::Ui;
use k8s_openapi::api::rbac::v1::{ClusterRole, PolicyRule, Role};

use crate::views::detail::fields::{ITEM_LIMIT, NONE, heading};
use crate::views::detail::lines::{first_of, line, list, none, selector, table};
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
    let words = |items: Option<&Vec<String>>| list(items.into_iter().flatten().map(String::as_str));
    [
        resources(rule),
        words(rule.non_resource_urls.as_ref()),
        words(rule.resource_names.as_ref()),
        words(Some(&rule.verbs)),
    ]
}

/// Each resource of a rule with each of its groups: `pods`, `deployments.apps`. Only the
/// names that show are made: a rule of 1.5 MB can name 100,000 resources and 100,000
/// groups, and their product does not fit in memory.
fn resources(rule: &PolicyRule) -> String {
    let (resources, groups) =
        (rule.resources.as_deref().unwrap_or_default(), rule.api_groups.as_deref().unwrap_or_default());
    let pairs = resources.iter().flat_map(|resource| groups.iter().map(move |group| (resource, group)));
    let shown: Vec<String> = pairs
        .take(ITEM_LIMIT)
        .map(|(resource, group)| if group.is_empty() { resource.clone() } else { format!("{resource}.{group}") })
        .collect();
    first_of(&shown, resources.len().saturating_mul(groups.len()))
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

    /// 100,000 resources and 100,000 groups fit in one object of the API. Their product,
    /// as a list of texts, would stop the frame and take each byte of the memory.
    #[test]
    fn a_rule_with_a_product_of_resources_and_groups_too_large_for_the_memory_shows_its_first_names() {
        let names = |prefix: &str| (0..100_000).map(|number| format!("{prefix}{number}")).collect::<Vec<_>>();
        let rule = PolicyRule {
            api_groups: Some(names("g")),
            resources: Some(names("r")),
            verbs: vec!["get".to_owned()],
            ..PolicyRule::default()
        };
        let text = resources(&rule);
        assert!(text.starts_with("r0.g0, r0.g1, r0.g2"), "{text}");
        assert!(text.ends_with("r0.g99 and 9,999,999,900 more"), "{text}");
    }
}
