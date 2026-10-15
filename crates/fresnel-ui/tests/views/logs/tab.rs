//! The tab of a log view: the action that opens it, its title with the color of its
//! cluster, the state of each source, its options and its close.

use std::time::Duration;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::CoreError;
use fresnel_core::app::Command;
use fresnel_core::kinds::Kind;
use fresnel_core::logs::{Containers, LogOptions, LogRequest, SourceState};
use fresnel_core::net::ErrorClass;
use fresnel_core::store::ObjectKey;

use super::{line, log_view, marker, opened, rows, show, shown, source, uid, until_row, with_options};
use crate::data::{live, pods};
use crate::fake::{SECOND, now, shows, until, view, window};
use crate::kinds::scene::deployments;
use crate::workloads::scene::{allowed, open, press};

const WEB: &str = "web-6888b778b6-75md7";

#[test]
fn the_logs_action_of_a_pod_opens_a_tab_that_shows_the_state_of_each_source_and_its_lines() {
    let (mut window, request) = opened(WEB);
    let expected = LogRequest {
        cluster: fresnel_core::cluster::ClusterId::new("ams-prod"),
        kind: Kind::Pod,
        key: ObjectKey::new("scene", WEB),
        uid: uid(WEB),
        options: LogOptions::default(),
    };
    assert_eq!(request, expected, "the default container, the last 1,000 lines, and follow");
    let title = format!("Logs {WEB}");
    assert!(window.query_by_role_and_label(Role::Button, &title).is_some(), "the tab has the name of the pod");
    assert!(shows(&window, "The view starts."), "no view before the core gives one");

    let failed = SourceState::Reconnecting {
        reason: CoreError::new(ErrorClass::Network, "the stream broke"),
        at: now() + 5 * SECOND,
    };
    let sources = vec![source(WEB, "web", SourceState::Streaming), source(WEB, "side", failed)];
    let lines =
        vec![line(3.0, 0, "GET /health 200"), marker(2.0, 1, "the stream started again"), line(1.0, 1, "ready")];
    show(&mut window, vec![log_view(&request, sources, lines)]);
    for text in ["web-6888b778b6-75md7/web", "streaming", "reconnecting at 08:00:05 after: the stream broke"] {
        assert!(shows(&window, text), "{text}");
    }
    until_row(&mut window, "ready");
    assert_eq!(
        rows(&window),
        [
            "07:59:57.000 web-6888b778b6-75md7/web GET /health 200",
            "07:59:58.000 web-6888b778b6-75md7/side the stream started again",
            "07:59:59.000 web-6888b778b6-75md7/side ready"
        ],
        "the time and the source of each line"
    );
    window.get_by_label(&format!("Close {title}")).click();
    window.run();
    assert_eq!(shown(&window), [], "the core stops the view");
    assert!(window.query_by_role_and_label(Role::Button, &title).is_none());
    assert!(shows(&window, "0 errors   0 warnings"), "the dock shows the journal again");
}

#[test]
fn a_workload_view_tells_how_far_it_knows_its_pods_and_what_it_lost() {
    let objects = deployments();
    let mut window = window(view(vec![allowed("ams-prod", &objects)]));
    open(&mut window, "Deployments", "web");
    window.get_by_role_and_label(Role::Button, "Logs").click();
    window.run();
    let request = shown(&window).into_iter().next().expect("a view");
    assert_eq!((request.kind, request.key.name.as_ref()), (Kind::Deployment, "web"));
    assert!(window.query_by_role_and_label(Role::Button, "Logs Deployment web").is_some());

    let mut view = log_view(&request, Vec::new(), Vec::new());
    view.pods = fresnel_core::logs::PodsState::Loading;
    show(&mut window, vec![view.clone()]);
    assert!(shows(&window, "The pods are not loaded yet."));

    let sources = (0..8).map(|index| source(&format!("web-{index}"), "web", SourceState::Streaming)).collect();
    let view = fresnel_core::logs::LogView {
        pods: fresnel_core::logs::PodsState::Following,
        dropped: 12_000,
        unstreamed: 2,
        restarts: 1,
        ..log_view(&request, sources, vec![line(1.0, 7, "from the last pod")])
    };
    show(&mut window, vec![view]);
    for text in [
        "12,000 older lines left the view at its limit.",
        "2 containers of the view have no stream: the view has its most streams.",
        "The view started again 1 time after an internal error of the app; earlier lines are gone.",
        "and 2 more",
    ] {
        assert!(shows(&window, text), "{text}");
    }
    until_row(&mut window, "web-7/web from the last pod");
}

#[test]
fn a_container_that_restarted_has_its_previous_logs_and_the_tab_tells_them() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    open(&mut window, "Pods", "crash");
    assert!(shows(&window, "Logs of crash"));
    press(&mut window, "Previous logs of crash");
    let request = shown(&window).into_iter().next().expect("a view");
    let expected =
        LogOptions { containers: Containers::Named("crash".into()), previous: true, ..LogOptions::default() };
    assert_eq!(request.options, expected);
    assert!(window.query_by_role_and_label(Role::Button, "Logs crash · crash (previous)").is_some());
    let previous = window.get_by_role_and_label(Role::CheckBox, "Previous");
    assert!(!previous.accesskit_node().is_disabled(), "the container restarted");

    super::open_pod(&mut window, WEB);
    assert!(shows(&window, "Logs of web") && !shows(&window, "Previous logs of web"), "web did not restart");
}

#[test]
fn new_options_name_a_new_view_and_the_previous_instance_needs_a_restart() {
    let (mut window, request) = opened(WEB);
    show(&mut window, vec![log_view(&request, vec![source(WEB, "web", SourceState::Streaming)], Vec::new())]);
    let previous = window.get_by_role_and_label(Role::CheckBox, "Previous");
    assert!(previous.accesskit_node().is_disabled(), "no container of the pod restarted");
    press(&mut window, "Tail of the view");
    press(&mut window, "Last 100");
    assert_eq!(shown(&window), [with_options(&request, |options| options.tail = Some(100))]);
    press(&mut window, "Since of the view");
    press(&mut window, "Last 15 min");
    let since = Some(fresnel_core::logs::Since::Last(Duration::from_secs(900)));
    assert_eq!(shown(&window), [with_options(&request, |options| (options.tail, options.since) = (Some(100), since))]);
    assert!(shows(&window, "The view starts."), "the tab waits for the new view");
}

#[test]
fn a_view_of_a_cluster_that_is_not_connected_says_why_and_a_pod_that_is_gone_says_it() {
    let (mut window, request) = opened(WEB);
    let mut view = window.state().view.clone();
    if let Some(row) = view.clusters.first_mut() {
        row.data = None;
        row.link = fresnel_core::cluster::LinkState::Failed(CoreError::new(ErrorClass::Network, "no route"));
    }
    window.state_mut().view = view;
    until(&mut window, "the note of the cluster", |window| {
        shows(window, "ams-prod is not connected: no route. The view starts when it connects.")
    });

    let (mut window, request_again) = opened(WEB);
    assert_eq!(request_again, request);
    let mut gone =
        log_view(&request, vec![source(WEB, "web", SourceState::Ended { reason: "the pod is gone".into() })], vec![]);
    gone.pods = fresnel_core::logs::PodsState::Gone;
    show(&mut window, vec![gone]);
    assert!(
        shows(&window, "The pod is gone. The view keeps what it read.") && shows(&window, "ended: the pod is gone")
    );
}

/// The core gets the view of each tab, and the same request opens its tab again.
#[test]
fn each_tab_names_its_view_to_the_core_and_the_same_request_opens_its_tab_again() {
    let (mut window, first) = opened(WEB);
    super::open_pod(&mut window, "db-0");
    window.get_by_role_and_label(Role::Button, "Logs").click();
    window.run();
    let both = shown(&window);
    assert_eq!(both.len(), 2);
    assert_eq!(both.first(), Some(&first));
    super::open_pod(&mut window, WEB);
    window.get_by_role_and_label(Role::Button, "Logs").click();
    window.run();
    assert_eq!(shown(&window), both, "the tab of the view opens again");
    let commands = window.state().sent.iter().filter(|command| matches!(command, Command::ShowLogs(_))).count();
    assert_eq!(commands, 2, "a command only for a change");
}
