//! The objects, the permissions and the jobs of the workload tests. The workloads come from
//! the scene of the indexes on k3s, as the pods of the fake core, so their pods are there.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::{AccessView, ClusterRow, Command, JobView, JobsView, RulesState};
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Describe;
use fresnel_core::ops::access::Rules;
use fresnel_core::ops::{JobState, Operation, Order, Outcome, Report, Step, StepState, Target};
use fresnel_core::store::{ObjectKey, StoreState};
use k8s_openapi::api::authorization::v1::SelfSubjectRulesReview;
use k8s_openapi::api::batch::v1::Job;
use serde_json::json;

use crate::data::{held, scene, store};
use crate::fake::{Window, until, view, window};
use crate::kinds::in_panel;
use crate::kinds::scene::{live, uid_of};

/// The rules of a user who can do each verb on each resource in the namespace `scene`,
/// but these verbs on these resources (`(verb, resource)` pairs) are missing.
pub(crate) fn rules_without(missing: &[(&str, &str)]) -> Rules {
    let verbs = ["get", "list", "watch", "create", "update", "patch", "delete"];
    let resources = [
        ("", "pods"),
        ("", "pods/eviction"),
        ("", "replicationcontrollers"),
        ("", "replicationcontrollers/scale"),
        ("apps", "deployments"),
        ("apps", "deployments/scale"),
        ("apps", "statefulsets"),
        ("apps", "statefulsets/scale"),
        ("apps", "daemonsets"),
        ("apps", "replicasets"),
        ("apps", "replicasets/scale"),
        ("apps", "controllerrevisions"),
        ("batch", "jobs"),
        ("batch", "cronjobs"),
    ];
    let mut rules = Vec::new();
    for (group, resource) in resources {
        let allowed: Vec<&str> = verbs.iter().copied().filter(|verb| !missing.contains(&(*verb, resource))).collect();
        rules.push(json!({"verbs": allowed, "apiGroups": [group], "resources": [resource]}));
    }
    let review: SelfSubjectRulesReview = serde_json::from_value(json!({
        "spec": {"namespace": "scene"},
        "status": {"resourceRules": rules, "nonResourceRules": [], "incomplete": false},
    }))
    .expect("a review");
    Rules::of("scene", review).expect("rules")
}

/// The permissions of a cluster with these rules for the namespace `scene`.
pub(crate) fn access(rules: Rules) -> Arc<AccessView> {
    let rules = BTreeMap::from([(Arc::from("scene"), RulesState::Read(Arc::new(rules)))]);
    Arc::new(AccessView { rules, checks: BTreeMap::new() })
}

/// A live cluster with these objects of a kind, the Jobs of the scene for the panel of a
/// CronJob, and these permissions.
pub(crate) fn cluster<K: Describe>(name: &str, objects: &[Arc<K>], permissions: Arc<AccessView>) -> ClusterRow {
    let row = live(name, objects);
    let mut data = row.data.clone().expect("data");
    data.objects = data.objects.with(held(store(StoreState::Live, &scene::<Job>())));
    data.access = permissions;
    ClusterRow { data: Some(data), ..row }
}

/// A cluster where the user can do each action.
pub(crate) fn allowed<K: Describe>(name: &str, objects: &[Arc<K>]) -> ClusterRow {
    cluster(name, objects, access(rules_without(&[])))
}

/// The window with the page of a kind and the panel of one of its objects open.
pub(crate) fn opened<K: Describe>(clusters: Vec<ClusterRow>, name: &str) -> Window {
    let mut window = window(view(clusters));
    open(&mut window, K::TITLE, name);
    window
}

/// Opens the page of a kind and the panel of an object of it. The filter of the page puts
/// its row on the screen: the table draws only the rows there.
pub(crate) fn open(window: &mut Window, page: &str, name: &str) {
    crate::fake::open_page(window, page);
    window.get_by_role(Role::TextInput).click();
    window.run();
    window.get_by_role(Role::TextInput).type_text(name);
    // The rows of the filter come from the worker thread: a click on the rows of before
    // could hit an other row.
    until(window, "the rows of the filter", |window| {
        window.query_by_label("updating…").is_none() && window.query_by_role_and_label(Role::Label, name).is_some()
    });
    window.get_by_role_and_label(Role::Label, name).click();
    until(window, "the panel", |window| in_panel(window, name));
}

/// Clicks the first widget with this label in the panel, as a screen reader does: a link
/// that wraps to a second line has a box whose middle can be on an other link.
pub(crate) fn press(window: &mut Window, label: &str) {
    window.query_all_by_label(label).next().expect("a widget with the label").click_accesskit();
    window.run();
}

/// Holds the pointer over a widget until its hint shows: with no wait, as the hint of a
/// person who holds the pointer still.
pub(crate) fn hover(window: &mut Window, role: Role, label: &str) {
    window.ctx.all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
    window.get_by_role_and_label(role, label).hover();
    for _ in 0..4 {
        window.step();
    }
    window.run();
}

/// The target of an object of the scene.
pub(crate) fn target<K: Describe>(cluster: &str, objects: &[Arc<K>], name: &str) -> Target {
    let namespace = objects
        .iter()
        .find(|object| object.metadata().name.as_deref() == Some(name))
        .and_then(|object| object.metadata().namespace.clone())
        .unwrap_or_default();
    Target {
        cluster: ClusterId::new(cluster),
        kind: K::ID,
        key: ObjectKey::new(&namespace, name),
        uid: uid_of(objects, name),
    }
}

/// The orders of each `Command::Run` that the views sent.
pub(crate) fn runs(window: &Window) -> Vec<Order> {
    let sent = window.state().sent.iter();
    sent.filter_map(|command| match command {
        Command::Run(orders) => Some(orders.clone()),
        _ => None,
    })
    .flatten()
    .collect()
}

/// A job of the core with this number, order and state, and steps.
pub(crate) fn job(
    number: u64,
    target: Target,
    operation: Operation,
    state: JobState,
    steps: &[(&str, StepState)],
) -> Arc<JobView> {
    let order = Order::new(target, operation).expect("an order");
    let steps = steps.iter().map(|(text, state)| Step { text: (*text).to_owned(), state: state.clone() }).collect();
    Arc::new(JobView { number, order, report: Report { state, steps, outcome: Outcome::None } })
}

/// The list of jobs of the core.
pub(crate) fn jobs(jobs: Vec<Arc<JobView>>, dropped: u64) -> Arc<JobsView> {
    Arc::new(JobsView { jobs, dropped })
}
