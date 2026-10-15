//! The sections of a MutatingWebhookConfiguration and of a ValidatingWebhookConfiguration
//! (`research/01` section 3.4.5): each webhook with its client, its policies, its selectors,
//! its rules and its match conditions. The Service of a client is a link.
//!
//! The two kinds have two types of webhook with the same fields; a mutating webhook adds
//! its reinvocation policy. `Hook` holds what the view reads of each one.

use egui::{Label, RichText, Ui};
use fresnel_core::kinds::Kind;
use k8s_openapi::api::admissionregistration::v1::{
    MatchCondition, MutatingWebhookConfiguration, RuleWithOperations, ValidatingWebhookConfiguration,
    WebhookClientConfig,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;

use crate::format::short;
use crate::views::detail::fields::{ITEM_LIMIT, NONE, VALUE_LIMIT, field, heading, more};
use crate::views::detail::lines::{line, list, narrows, none, selector};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;

/// What the view shows of one webhook of either kind.
struct Hook<'a> {
    name: &'a str,
    client: &'a WebhookClientConfig,
    failure: Option<&'a str>,
    matching: Option<&'a str>,
    side_effects: &'a str,
    timeout: Option<i32>,
    versions: &'a [String],
    namespaces: Option<&'a LabelSelector>,
    objects: Option<&'a LabelSelector>,
    rules: &'a [RuleWithOperations],
    conditions: &'a [MatchCondition],
    /// The reinvocation policy of a mutating webhook. `None` for a validating one.
    reinvocation: Option<Option<&'a str>>,
}

pub(crate) fn mutating(ui: &mut Ui, cx: &Context<'_>, configuration: &MutatingWebhookConfiguration) -> Option<Link> {
    let hooks = configuration.webhooks.iter().flatten().map(|hook| Hook {
        name: &hook.name,
        client: &hook.client_config,
        failure: hook.failure_policy.as_deref(),
        matching: hook.match_policy.as_deref(),
        side_effects: &hook.side_effects,
        timeout: hook.timeout_seconds,
        versions: &hook.admission_review_versions,
        namespaces: hook.namespace_selector.as_ref(),
        objects: hook.object_selector.as_ref(),
        rules: hook.rules.as_deref().unwrap_or_default(),
        conditions: hook.match_conditions.as_deref().unwrap_or_default(),
        reinvocation: Some(hook.reinvocation_policy.as_deref()),
    });
    show(ui, cx, &hooks.collect::<Vec<_>>())
}

pub(crate) fn validating(
    ui: &mut Ui,
    cx: &Context<'_>,
    configuration: &ValidatingWebhookConfiguration,
) -> Option<Link> {
    let hooks = configuration.webhooks.iter().flatten().map(|hook| Hook {
        name: &hook.name,
        client: &hook.client_config,
        failure: hook.failure_policy.as_deref(),
        matching: hook.match_policy.as_deref(),
        side_effects: &hook.side_effects,
        timeout: hook.timeout_seconds,
        versions: &hook.admission_review_versions,
        namespaces: hook.namespace_selector.as_ref(),
        objects: hook.object_selector.as_ref(),
        rules: hook.rules.as_deref().unwrap_or_default(),
        conditions: hook.match_conditions.as_deref().unwrap_or_default(),
        reinvocation: None,
    });
    show(ui, cx, &hooks.collect::<Vec<_>>())
}

fn show(ui: &mut Ui, cx: &Context<'_>, hooks: &[Hook<'_>]) -> Option<Link> {
    heading(ui, cx.palette, "WEBHOOKS");
    if hooks.is_empty() {
        none(ui, cx);
    }
    let mut clicked = None;
    for hook in hooks.iter().take(ITEM_LIMIT) {
        ui.label(RichText::new(short(hook.name, VALUE_LIMIT)).strong());
        field(ui, cx.palette, "Client", |ui| clicked = client(ui, cx, hook.client).or(clicked.take()));
        line(ui, cx, "Failure policy", Some(hook.failure.unwrap_or("Fail").to_owned()));
        line(ui, cx, "Match policy", Some(hook.matching.unwrap_or("Equivalent").to_owned()));
        line(ui, cx, "Side effects", Some(hook.side_effects.to_owned()));
        line(ui, cx, "Timeout", Some(format!("{} s", hook.timeout.unwrap_or(10))));
        line(ui, cx, "Review versions", Some(hook.versions.join(", ")));
        line(ui, cx, "Namespaces", Some(each(hook.namespaces, "each namespace")));
        line(ui, cx, "Objects", Some(each(hook.objects, "each object")));
        if let Some(policy) = hook.reinvocation {
            line(ui, cx, "Reinvocation", Some(policy.unwrap_or("Never").to_owned()));
        }
        field(ui, cx.palette, "Rules", |ui| {
            if hook.rules.is_empty() {
                ui.label(NONE);
            }
            for each in hook.rules.iter().take(ITEM_LIMIT) {
                ui.add(Label::new(short(&rule(each), VALUE_LIMIT)).wrap());
            }
            more(ui, cx.palette, hook.rules.len());
        });
        if !hook.conditions.is_empty() {
            field(ui, cx.palette, "Conditions", |ui| {
                for condition in hook.conditions.iter().take(ITEM_LIMIT) {
                    let text = format!("{}: {}", condition.name, condition.expression);
                    ui.add(Label::new(short(&text, VALUE_LIMIT)).wrap());
                }
                more(ui, cx.palette, hook.conditions.len());
            });
        }
        ui.add_space(8.0);
    }
    more(ui, cx.palette, hooks.len());
    ui.add_space(10.0);
    clicked
}

/// The Service of a client as a link with its port and its path, or its URL.
fn client(ui: &mut Ui, cx: &Context<'_>, client: &WebhookClientConfig) -> Option<Link> {
    if let Some(service) = &client.service {
        let port = service.port.unwrap_or(443);
        let path = service.path.as_deref().unwrap_or_default();
        let text = format!("{}/{}:{port}{path}", service.namespace, service.name);
        return cx.link(ui, &text, Kind::Service, &service.namespace, &service.name);
    }
    ui.label(short(client.url.as_deref().unwrap_or(NONE), VALUE_LIMIT));
    None
}

/// The text of a selector, or `all` for a selector that selects each object.
fn each(selected: Option<&LabelSelector>, all: &str) -> String {
    selected.filter(|labels| narrows(labels)).map_or_else(|| all.to_owned(), |labels| selector(Some(labels)))
}

/// "CREATE, UPDATE of pods, deployments/scale in core, apps (v1), scope *".
fn rule(rule: &RuleWithOperations) -> String {
    let words = |items: &Option<Vec<String>>| list(items.iter().flatten().map(String::as_str));
    let groups = list(rule.api_groups.iter().flatten().map(|group| if group.is_empty() { "core" } else { group }));
    format!(
        "{} of {} in {groups} ({}), scope {}",
        words(&rule.operations),
        words(&rule.resources),
        words(&rule.api_versions),
        rule.scope.as_deref().unwrap_or("*")
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_rule_names_its_operations_resources_groups_and_scope() {
        let parsed: RuleWithOperations = serde_json::from_value(json!({"apiGroups": ["", "apps"],
            "apiVersions": ["v1"], "operations": ["CREATE", "UPDATE"], "resources": ["pods", "deployments/scale"]}))
        .expect("a rule");
        assert_eq!(rule(&parsed), "CREATE, UPDATE of pods, deployments/scale in core, apps (v1), scope *");
        assert_eq!(each(None, "each namespace"), "each namespace");
        assert_eq!(each(Some(&LabelSelector::default()), "each namespace"), "each namespace");
    }
}
