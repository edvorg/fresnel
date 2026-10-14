//! The detail view of each workload kind (plan section 11.4): the content of Headlamp and
//! Lens, and the links to the objects that a spec names.

use std::collections::BTreeSet;

use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::Command;
use fresnel_core::kinds::Kind;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Pod, ReplicationController};

use super::scene::{allowed, opened, press};
use crate::data::{of_kinds, pods, scene};
use crate::fake::{absent, shows, until};
use crate::kinds::in_panel;

/// The texts of the panel that are not there.
fn missing(window: &crate::fake::Window, texts: &[&str]) -> Vec<String> {
    texts.iter().filter(|text| !in_panel(window, text)).map(|text| (*text).to_owned()).collect()
}

/// A pod: its details, each container with its state, restarts and last state, the
/// environment with a link to the Secret of a key, and the volumes with their objects.
#[test]
fn a_pod_shows_its_containers_with_their_state_and_its_volumes_with_links() {
    let mut window = opened::<Pod>(vec![allowed("ams-prod", &pods())], "web-6888b778b6-75md7");
    let texts = [
        "DETAILS",
        "Pod IPs",
        "QoS class",
        "Service account",
        "web",
        "CONTAINERS",
        "container",
        "rancher/mirrored-pause:3.6",
        "http 80/TCP",
        "PASSWORD",
        "key password of",
        "Secret web-secret",
        "ConfigMap web-env",
        "/config from config (rw)",
        "VOLUMES",
        "ConfigMap web-config",
        "Secret web-tls",
        "token of the ServiceAccount",
    ];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    assert!(window.query_all_by_label_contains("Running since 2027-01-15").next().is_some());

    // The link of a key goes to the panel of its Secret, which the core then reads.
    press(&mut window, "Secret web-secret");
    until(&mut window, "the page of the Secrets", |window| {
        window.state().sent.contains(&Command::ShowKinds(BTreeSet::from([Kind::Secret])))
    });
    assert!(shows(&window, "The Secret web-secret of ams-prod opens when its store has a complete list."));
}

#[test]
fn a_pod_that_restarts_shows_its_wait_its_restarts_and_its_last_state() {
    let window = opened::<Pod>(vec![allowed("ams-prod", &pods())], "crash");
    assert!(window.query_all_by_label_contains("Waiting: CrashLoopBackOff: back-off 20s").next().is_some());
    assert_eq!(missing(&window, &["Restarts", "2", "Last state"]), Vec::<String>::new());
    let last = "Terminated: Error (exit code 1), finished 2027-01-15";
    assert!(window.query_all_by_label_contains(last).next().is_some(), "the last state");
}

/// A Deployment: the replicas, the strategy, its pods with their state as links, its
/// revisions on request, and its template.
#[test]
fn a_deployment_shows_its_replicas_strategy_pods_revisions_and_template() {
    let deployments: Vec<_> = scene::<Deployment>();
    let window = opened::<Deployment>(vec![allowed("ams-prod", &deployments)], "web");
    let texts = [
        "Replicas",
        "2 desired, 2 ready, 2 up to date, 2 available, 2 total",
        "RollingUpdate (max surge 25%, max unavailable 25%)",
        "app=web",
        "600 s",
        "10 revisions",
        "REPLICASETS",
        "PODS",
        "web-6888b778b6-75md7",
        "1/1",
        "REVISIONS",
        "Read the revisions",
        "POD TEMPLATE",
        "Secret web-secret",
    ];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    assert!(window.query_all_by_label("Running").count() >= 2, "the state of each pod");
}

#[test]
fn a_stateful_set_and_a_daemon_set_show_their_strategy_and_their_pods() {
    let window = opened::<StatefulSet>(vec![allowed("ams-prod", &scene::<StatefulSet>())], "db");
    let texts = ["Update strategy", "RollingUpdate", "Pod management", "OrderedReady", "db-0", "db-1", "REVISIONS"];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());

    let window = opened::<DaemonSet>(vec![allowed("ams-prod", &scene::<DaemonSet>())], "agent");
    let found = window.query_all_by_label_contains(" desired, ").next().map(|node| node.value().unwrap_or_default());
    assert!(found.is_some_and(|text| text.contains("misscheduled")), "the counts of the nodes");
    assert_eq!(missing(&window, &["Update strategy", "agent-8djvd", "REVISIONS"]), Vec::<String>::new());
}

#[test]
fn a_replica_set_and_a_replication_controller_show_their_replicas_and_selector() {
    let window = opened::<ReplicaSet>(vec![allowed("ams-prod", &scene::<ReplicaSet>())], "web-6888b778b6");
    assert_eq!(missing(&window, &["Replicas", "Selector", "PODS", "web-6888b778b6-p2twl"]), Vec::<String>::new());
    assert!(!in_panel(&window, "REVISIONS"), "a ReplicaSet has no revisions");

    let window =
        opened::<ReplicationController>(vec![allowed("ams-prod", &of_kinds::<ReplicationController>())], "legacy");
    assert_eq!(missing(&window, &["Replicas", "Selector", "Observed", "PODS", "POD TEMPLATE"]), Vec::<String>::new());
}

/// A Job: its state, its counts, its times and its time to live.
#[test]
fn a_job_shows_its_state_completions_times_and_pods() {
    let window = opened::<Job>(vec![allowed("ams-prod", &scene::<Job>())], "once");
    let texts = ["State", "Complete", "Completions", "Parallelism", "Backoff limit", "Started", "Duration", "PODS"];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    let window = opened::<Job>(vec![allowed("ams-prod", &scene::<Job>())], "fails");
    assert!(window.query_all_by_label_contains("Failed: BackoffLimitExceeded").next().is_some());
}

/// A CronJob: its schedule and policy, and its last runs with the state of each Job, from
/// the store of the Jobs that its page holds.
#[test]
fn a_cron_job_shows_its_schedule_and_its_last_runs_with_their_state() {
    let mut window = opened::<CronJob>(vec![allowed("ams-prod", &scene::<CronJob>())], "tick");
    assert!(window.state().sent.contains(&Command::ShowKinds(BTreeSet::from([Kind::CronJob, Kind::Job]))));
    let texts =
        ["Schedule", "Concurrency", "Allow", "History limits", "LAST RUNS", "tick-manual", "Complete", "POD TEMPLATE"];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    assert_eq!(absent(&window, &["Last success"]), Vec::<&str>::new());
    press(&mut window, "tick-manual");
    until(&mut window, "the panel of the Job", |window| in_panel(window, "Completions"));
}
