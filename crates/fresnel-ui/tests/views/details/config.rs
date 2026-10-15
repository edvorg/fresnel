//! The views of the Config kinds besides the keys: an autoscaler, a disruption budget, a
//! quota and a limit range, the two classes, a Lease and the webhook configurations, whose
//! page has the longest title of a kind.

use egui_kittest::kittest::Queryable as _;
use k8s_openapi::api::admissionregistration::v1::{MutatingWebhookConfiguration, ValidatingWebhookConfiguration};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::coordination::v1::Lease;
use k8s_openapi::api::core::v1::{LimitRange, ResourceQuota};
use k8s_openapi::api::node::v1::RuntimeClass;
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use k8s_openapi::api::scheduling::v1::PriorityClass;
use serde_json::json;

use super::{and, follow, shows_part};
use crate::data::{changed_object, of_kinds};
use crate::fake::until;
use crate::kinds::scene::live;
use crate::kinds::{in_panel, missing};
use crate::workloads::scene::opened;

#[test]
fn an_autoscaler_shows_its_target_replicas_and_metrics_and_its_target_links_its_panel() {
    let cluster = and(live("ams-prod", &of_kinds::<HorizontalPodAutoscaler>()), &of_kinds::<Deployment>());
    let mut window = opened::<HorizontalPodAutoscaler>(vec![cluster], "web");
    let texts = ["Target", "Deployment web", "Replicas", "1 to 3", "METRICS", "<unknown>", "80%", "CONDITIONS"];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    assert!(in_panel(&window, "resource cpu on pods (as a percentage of request)"));
    assert!(in_panel(&window, "ScalingActive"), "the conditions of the status");

    follow(&mut window, "Deployment web");
    until(&mut window, "the panel of the Deployment", |window| in_panel(window, "POD TEMPLATE"));
}

#[test]
fn a_disruption_budget_tells_whether_an_eviction_can_go_now() {
    let budgets = of_kinds::<PodDisruptionBudget>();
    let window = opened::<PodDisruptionBudget>(vec![live("ams-prod", &budgets)], "web");
    assert_eq!(missing(&window, &["Min available", "1", "Selector", "app=web"]), Vec::<String>::new());
    assert!(shows_part(&window, "none allowed: an eviction of a pod waits"));
    assert!(shows_part(&window, "1 current, 1 desired, of 1 expected pods"));
    let window = opened::<PodDisruptionBudget>(vec![live("ams-prod", &budgets)], "db");
    assert_eq!(missing(&window, &["Max unavailable", "50%", "1 allowed"]), Vec::<String>::new());
}

#[test]
fn a_quota_shows_the_use_of_each_resource_against_its_hard_limit() {
    let window = opened::<ResourceQuota>(vec![live("ams-prod", &of_kinds::<ResourceQuota>())], "quota");
    let texts = ["Scopes", "each object", "USE", "pods", "6", "20", "30%", "requests.cpu", "70m", "4", "2%"];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    assert_eq!(missing(&window, &["limits.memory", "448Mi", "8Gi", "5%"]), Vec::<String>::new());
}

#[test]
fn a_limit_range_shows_each_limit_of_each_resource() {
    let window = opened::<LimitRange>(vec![live("ams-prod", &of_kinds::<LimitRange>())], "limits");
    let texts = ["LIMITS", "Container", "cpu", "10m", "memory", "16Mi", "64Mi", "Default request", "Default limit"];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
}

#[test]
fn the_two_classes_show_their_values() {
    let window = opened::<PriorityClass>(vec![live("ams-prod", &of_kinds::<PriorityClass>())], "fresnel-low");
    let texts = ["Value", "-10", "Global default", "no", "Preemption", "Never", "The priority of the kinds test."];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    let classes = of_kinds::<RuntimeClass>();
    let gpu = changed_object(&*classes[0], |value| {
        value["overhead"] = json!({"podFixed": {"cpu": "250m", "memory": "120Mi"}});
        value["scheduling"] = json!({"nodeSelector": {"pool": "gpu"},
            "tolerations": [{"key": "gpu", "operator": "Exists", "effect": "NoSchedule"}]});
    });
    let window = opened::<RuntimeClass>(vec![live("ams-prod", &[gpu])], "fresnel-runc");
    let texts = ["Handler", "runc", "cpu 250m, memory 120Mi", "pool=gpu", "gpu:NoSchedule op=Exists"];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
}

/// The fixture is at 08:00:00. The holder renewed its lease of 15 s at 07:59:30.
#[test]
fn a_lease_shows_its_holder_and_when_it_ended() {
    let leases = of_kinds::<Lease>();
    let renewed = changed_object(&*leases[0], |value| {
        value["spec"]["renewTime"] = "2027-01-15T07:59:30.000000Z".into();
        value["spec"]["leaseTransitions"] = 2.into();
    });
    let window = opened::<Lease>(vec![live("ams-prod", &[renewed])], "leader");
    assert_eq!(missing(&window, &["Holder", "web-1", "Duration", "15 s", "Transitions", "2"]), Vec::<String>::new());
    assert!(shows_part(&window, "2027-01-15 07:59:30 (30 s ago)"), "the renewal");
    assert!(shows_part(&window, "ended 15 s ago: the holder did not renew it"));
}

#[test]
fn a_webhook_configuration_shows_each_webhook_with_its_service_and_rules() {
    let window = opened::<MutatingWebhookConfiguration>(
        vec![live("ams-prod", &of_kinds::<MutatingWebhookConfiguration>())],
        "fresnel-mutate",
    );
    let texts = ["WEBHOOKS", "mutate.fresnel.test", "kinds/nobody:443", "Ignore", "Equivalent", "None", "10 s"];
    let texts = [&texts[..], &["each namespace", "each object", "Reinvocation", "Never"]].concat();
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    assert!(shows_part(&window, "CREATE of widgets in fresnel.test (v1), scope *"));

    let window = opened::<ValidatingWebhookConfiguration>(
        vec![live("ams-prod", &of_kinds::<ValidatingWebhookConfiguration>())],
        "fresnel-validate",
    );
    assert_eq!(missing(&window, &["validate.fresnel.test", "second.fresnel.test"]), Vec::<String>::new());
    assert!(!in_panel(&window, "Reinvocation"), "a validating webhook changes no object");
    assert!(shows_part(&window, "DELETE of widgets in fresnel.test (v1), scope *"));
}

/// The longest title of a kind, with the panel open: the bar cuts the count of the rows,
/// which ran under the button "Problems" before.
#[test]
fn the_bar_of_a_page_with_a_long_title_cuts_its_count_before_its_buttons() {
    let hooks = of_kinds::<ValidatingWebhookConfiguration>();
    let window = opened::<ValidatingWebhookConfiguration>(vec![live("ams-prod", &hooks)], "fresnel-validate");
    let count = window.get_by_label("1 ValidatingWebhookConfiguration").rect();
    let problems = window.get_by_label("Problems").rect();
    assert!(count.right() <= problems.left(), "the count {count:?} ends before the button {problems:?}");
}
