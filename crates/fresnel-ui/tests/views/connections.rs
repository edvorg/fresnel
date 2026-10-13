//! The Connections view for each state of a cluster.

use std::time::Duration;

use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::{Command, ShellNote, Startup};
use fresnel_core::cluster::{ClusterId, LinkState};
use fresnel_core::net::credential::{CredentialFailure, CredentialState, PluginError, PluginFailure, RunReason};
use fresnel_core::net::{ChannelState, ChannelStatus};

use crate::fake::{
    SECOND, absent, channel, connected, connections_window, error, live_channels, live_status, new_channel, now, off,
    row, shows, status, valid, view,
};

#[test]
fn a_live_cluster_shows_its_credential_and_each_channel_with_its_numbers() {
    let mut live = live_status();
    live.watches.channels[0] = ChannelStatus {
        streams_open: 30,
        ping_time: Some(Duration::from_millis(11)),
        last_frame_age: Some(SECOND),
        connect_attempts: 4,
        last_error: Some("an old reset by the peer".to_owned()),
        ..channel(ChannelState::Live)
    };
    live.requests.channels[0].ping_time = Some(Duration::from_millis(1440));
    live.requests.channels[0].last_frame_age = Some(125 * SECOND);
    live.logs.channels[0].streams_open = 2;
    let window = connections_window(view(vec![connected("ams-prod", live)]));

    let card = ["ams-prod", "ams-prod.k8s.example:443  ·  plugin doctl", "live", "last answer 1 s ago"];
    let credential = ["Credential", "valid until 09:00:00 (in 1 h 0 min)", "1 plugin run"];
    let titles = ["Channel", "State", "Ping", "Streams", "Last answer", "Attempts", "Last error"];
    let watches = ["watches 1", "11 ms", "30 / 70", "1 s", "4", "an old reset by the peer"];
    let requests = ["requests 1", "1.4 s", "2 min 5 s"];
    let logs = ["log streams 1", "12 ms", "2 / 70", "3 s"];
    for texts in [&card[..], &credential, &titles, &watches, &requests, &logs] {
        assert_eq!(absent(&window, texts), Vec::<&str>::new());
    }
    assert_eq!(window.query_all_by_label("live").count(), 4, "the cluster and its 3 channels");
    assert!(shows(&window, "ams-prod: live"), "the status bar has the same state");
}

#[test]
fn a_cluster_that_connects_shows_the_attempt_and_that_no_answer_came() {
    let first_run =
        CredentialState::Running { reason: RunReason::Start, since: now() - 2 * SECOND, last_failure: None };
    let channels = [
        new_channel(ChannelState::Connecting { attempt: 1 }),
        new_channel(ChannelState::Verifying),
        new_channel(ChannelState::Connecting { attempt: 1 }),
    ];
    let window = connections_window(view(vec![connected("fra-stage", status(channels, Some(first_run)))]));

    let texts = [
        "connecting",
        "no answer yet",
        "the plugin runs for the first token, for 2 s",
        "connected, no answer yet",
        "0 / 70",
    ];
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
    assert_eq!(window.query_all_by_label("connecting, attempt 1").count(), 2);
    assert_eq!(window.query_all_by_label("—").count(), 9, "no ping, no answer and no error for 3 channels");
}

#[test]
fn a_cluster_with_no_connection_shows_the_error_and_the_time_to_the_next_attempt() {
    let lost = |failures, pause: u32, attempts, error: &str| ChannelStatus {
        since: now() - SECOND,
        last_frame_age: Some(48 * SECOND),
        connect_attempts: attempts,
        last_error: Some(error.to_owned()),
        ..channel(ChannelState::Backoff { failures, pause: pause * SECOND })
    };
    let channels = [
        lost(3, 4, 7, "connect: no answer in 10s"),
        ChannelStatus {
            last_error: Some("connect: connection refused".to_owned()),
            ..new_channel(ChannelState::Connecting { attempt: 5 })
        },
        lost(1, 1, 2, "the server sent no ping answer in 5s"),
    ];
    let window = connections_window(view(vec![connected("sgp-prod", status(channels, Some(valid())))]));

    let texts = [
        "no connection",
        "last answer 48 s ago",
        "3 failures, next attempt in 3 s",
        "7",
        "connect: no answer in 10s",
        "connecting, attempt 5",
        "connect: connection refused",
        "1 failure, next attempt in 0 s",
        "the server sent no ping answer in 5s",
    ];
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
    assert!(shows(&window, "sgp-prod: no connection"), "the status bar has the same state");
}

#[test]
fn a_failed_credential_shows_the_text_of_the_plugin_and_the_next_attempt() {
    let detail = "Error: Unable to initialize DigitalOcean API client: access token is required.";
    let plugin = PluginError::new("doctl", PluginFailure::Exit("exit status: 1".to_owned()), detail.to_owned());
    let failed = CredentialState::Failed {
        failure: CredentialFailure::Plugin(plugin.clone()),
        failures: 2,
        next_attempt: now() + 8 * SECOND,
    };
    let mut no_token = status([(); 3].map(|()| new_channel(ChannelState::WaitsForToken)), Some(failed));
    no_token.credential.as_mut().expect("a credential").plugin_runs = 2;
    no_token.credential.as_mut().expect("a credential").unauthorized = 3;
    let again = CredentialState::Running {
        reason: RunReason::Retry,
        since: now() - 5 * SECOND,
        last_failure: Some(CredentialFailure::Plugin(plugin)),
    };
    let refused =
        CredentialState::Failed { failure: CredentialFailure::Refused, failures: 1, next_attempt: now() + 2 * SECOND };
    let clusters = vec![
        connected("nyc-prod", no_token),
        connected("ams-prod", status(live_channels(), Some(again))),
        connected("fra-stage", status(live_channels(), Some(refused))),
    ];
    let window = connections_window(view(clusters));

    let stopped = "credential plugin `doctl`: stopped with exit status: 1";
    let texts = [
        "connected, waits for a token".to_owned(),
        format!("{stopped}. Failure 2 in sequence, next attempt in 8 s"),
        "2 plugin runs, 3 answers of HTTP 401".to_owned(),
        format!("the plugin runs again after a failure, for 5 s. The failure before: {stopped}"),
        "the server refused a new token of the credential plugin (HTTP 401). Failure 1 in sequence, next attempt in 2 s"
            .to_owned(),
    ];
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
    assert_eq!(window.query_all_by_label(detail).count(), 2, "the text of the plugin, also while it runs again");
    assert_eq!(window.query_all_by_label("no credential").count(), 3);
    assert!(shows(&window, "nyc-prod: no credential"));
}

#[test]
fn a_connect_that_did_not_start_shows_its_reason_and_a_bad_entry_shows_as_such() {
    let reason = "cannot read the certificate authority /Users/tester/ca.pem: No such file or directory";
    let unsupported = "the user entry `ci` has a static token; the app does not support that yet";
    let mut bad_entry = row("ci", LinkState::Failed(error(unsupported)));
    (bad_entry.server, bad_entry.auth) = (Err(error(unsupported)), None);
    let clusters = vec![row("late", LinkState::Failed(error(reason))), bad_entry, row("new", LinkState::Off)];
    let window = connections_window(view(clusters));

    let texts = [
        "failed",
        "The connect did not start: cannot read the certificate authority /Users/tester/ca.pem: No such file or directory",
        "the app cannot use the kubeconfig entry",
        "The connect did not start: the user entry `ci` has a static token; the app does not support that yet",
        "The connect starts.",
        "off",
    ];
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
    assert!(!shows(&window, "Channel"), "a cluster with no connection has no channel table");
}

#[test]
fn a_throttle_requests_that_wait_and_an_extra_channel_show() {
    let mut busy = live_status();
    busy.throttle.until = Some(now() + 30 * SECOND);
    busy.watches.waiting = 2;
    busy.logs.waiting = 1;
    busy.watches.channels.push(ChannelStatus { streams_open: 70, ..channel(ChannelState::Live) });
    busy.requests.channels.push(ChannelStatus { last_frame_age: Some(90 * SECOND), ..channel(ChannelState::Retired) });
    let window = connections_window(view(vec![connected("ams-prod", busy)]));

    let texts = [
        "The server throttles the app (HTTP 429) until 08:00:30.",
        "2 requests wait for a free stream of the watches.",
        "1 request waits for a free stream of the log streams.",
        "watches 2",
        "70 / 70",
        "requests 2",
        "closed, it had no stream",
    ];
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
}

#[test]
fn retry_now_sends_the_command_for_its_cluster_and_the_view_does_nothing_else() {
    let clusters = vec![connected("ams-prod", live_status()), row("late", LinkState::Failed(error("no server")))];
    let mut window = connections_window(view(clusters));
    assert_eq!(window.state().sent, []);

    window.get_by_label("Retry now: late").click();
    window.run();
    assert_eq!(window.state().sent, [Command::RetryNow(ClusterId::new("late"))]);
    window.get_by_label("Retry now: ams-prod").click();
    window.run();
    assert_eq!(window.state().sent.last(), Some(&Command::RetryNow(ClusterId::new("ams-prod"))));
    assert_eq!(window.state().sent.len(), 2);
}

#[test]
fn the_view_tells_why_it_has_no_card() {
    let texts = [
        (Startup::Waiting, vec![], "The login shell gives its environment. Then the clusters show."),
        (Startup::Environment, vec![], "The login shell gives its environment. Then the clusters show."),
        (Startup::Kubeconfig, vec![], "The kubeconfig files load."),
        (
            Startup::Ready,
            vec![],
            "The kubeconfig files have no context. The Journal tab lists the files that the app looked for.",
        ),
        (Startup::Ready, vec![off("ams-prod")], "No cluster is selected. Select a cluster in the top bar."),
    ];
    for (startup, clusters, text) in texts {
        let mut empty = view(clusters);
        empty.startup = startup;
        let window = connections_window(empty);
        assert!(shows(&window, text), "{startup:?}: {text}");
        assert!(!shows(&window, "Retry now"));
    }
}

#[test]
fn a_login_shell_that_gave_no_environment_shows_with_the_text_of_its_profile() {
    let mut broken = view(vec![connected("ams-prod", live_status())]);
    broken.shell_problem = Some(ShellNote {
        problem: "the login shell /bin/zsh gave no environment in 5s".to_owned(),
        detail: "compinit: insecure directories, run compaudit for list.".to_owned(),
    });
    let window = connections_window(broken);
    let texts = [
        "The login shell gave no environment",
        "the login shell /bin/zsh gave no environment in 5s",
        "The app looks for a credential plugin in its own PATH and in the fixed folders.",
        "compinit: insecure directories, run compaudit for list.",
        "ams-prod",
    ];
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
}

/// New data arrives between two frames. In the prototype, a view that kept a position
/// into older data stopped the UI thread.
#[test]
fn a_new_copy_with_other_clusters_and_other_channels_in_each_frame_draws() {
    let names: Vec<String> = (0..12).map(|index| format!("cluster-{index}")).collect();
    let mut window = connections_window(view(Vec::new()));
    for step in 0..40_usize {
        let clusters = names.iter().skip(step % 5).take(step % 9).enumerate().map(|(index, name)| {
            let mut status = live_status();
            status.watches.channels.truncate((step + index) % 2);
            status.logs.channels.extend((0..(step + index) % 4).map(|_| channel(ChannelState::Live)));
            match (step + index) % 4 {
                0 => off(name),
                1 => row(name, LinkState::Failed(error("no server"))),
                _ => connected(name, status),
            }
        });
        window.state_mut().view = view(clusters.collect());
        window.run();
    }
    assert!(shows(&window, "cluster-4"), "the last frame shows its clusters");
}
