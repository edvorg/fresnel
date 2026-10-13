use super::super::report::{Alpn, Channel, Dns, Environment, Version};
use super::*;

fn summary(min: f64, median: f64, max: f64) -> Option<Summary> {
    Some(Summary { min, median, max })
}

fn channel(purpose: &'static str, state: &str, last_error: Option<&str>) -> Channel {
    Channel {
        purpose,
        state: state.to_owned(),
        connections: 1,
        stream_limit: Some(100),
        stream_budget: 70,
        light_request_ms: Some(21.5),
        last_error: last_error.map(str::to_owned),
    }
}

fn good_cluster() -> Cluster {
    let plain = Connects {
        samples: 20,
        connected: 18,
        held: 1,
        held_percent: 5.0,
        failed: 1,
        tcp_ms: summary(11.0, 12.5, 30.25),
        tls_ms: summary(24.0, 26.0, 41.0),
        ..Connects::default()
    };
    let hedged = Connects {
        samples: 20,
        connected: 20,
        total_ms: summary(36.0, 39.5, 351.0),
        attempts: Some(21),
        hedge_delay_ms: Some(150.0),
        ..Connects::default()
    };
    let session = Session {
        credential: Some(Credential {
            plugin_runs: 1,
            first_token_ms: Some(812.4),
            state: "the credential is valid".to_owned(),
            detail: String::new(),
        }),
        live_ms: Some(905.0),
        channels: vec![
            channel("watches", "live", None),
            channel("requests", "live", None),
            channel("logs", "live", None),
        ],
        stream_limit: Some(100),
        ping_ms: summary(20.0, 21.0, 25.0),
        ping_error: Some("request channel: connection reset".to_owned()),
        version: Step::Done(Version { git_version: "v1.33.1".to_owned(), platform: "linux/amd64".to_owned() }),
        idle_close: Some(IdleClose {
            waited_s: 90.4,
            closed_after_s: Some(90.4),
            reason: Some("the server closed the connection".to_owned()),
            light_request_kept_open: true,
        }),
    };
    let measured = Measured {
        dns: Step::Done(Dns { time_ms: 2.1, addresses: vec!["10.0.0.1:443".to_owned(), "10.0.0.2:443".to_owned()] }),
        alpn: Alpn { http2: Step::Done(true), http1: Step::Done(true) },
        plain: Some(plain),
        hedged: Some(hedged),
        session: Step::Done(session),
    };
    Cluster {
        context: "ams".to_owned(),
        server: Some("ams.example.test:443".to_owned()),
        auth: Some("plugin doctl".to_owned()),
        plugin: Some(Plugin {
            command: "doctl".to_owned(),
            folder: Some("/opt/homebrew/bin".to_owned()),
            in_process_path: false,
            search_ms: 0.2,
        }),
        result: Step::Done(measured),
    }
}

/// A cluster whose plugin fails, with a server that has no HTTP/2.
fn bad_cluster() -> Cluster {
    let session = Session {
        credential: Some(Credential {
            plugin_runs: 2,
            first_token_ms: None,
            state: "credential plugin `login`: stopped with exit status: 7".to_owned(),
            detail: "Error: no account\nRun `login init`.".to_owned(),
        }),
        live_ms: None,
        channels: vec![channel("watches", "pause after 3 failures", Some("no connection after 1s and 2 attempts"))],
        stream_limit: None,
        ping_ms: None,
        ping_error: None,
        version: Step::failed("credential plugin `login`: stopped with exit status: 7"),
        idle_close: None,
    };
    let measured = Measured {
        dns: Step::Done(Dns { time_ms: 0.0, addresses: vec!["127.0.0.1:6443".to_owned()] }),
        alpn: Alpn { http2: Step::failed("the server did not select Http2"), http1: Step::Done(true) },
        plain: None,
        hedged: None,
        session: Step::Done(session),
    };
    Cluster {
        context: "old".to_owned(),
        server: Some("127.0.0.1:6443".to_owned()),
        auth: Some("plugin login".to_owned()),
        plugin: Some(Plugin { command: "login".to_owned(), folder: None, in_process_path: false, search_ms: 0.1 }),
        result: Step::Done(measured),
    }
}

fn unusable_cluster() -> Cluster {
    Cluster {
        context: "token".to_owned(),
        server: None,
        auth: None,
        plugin: None,
        result: Step::failed("a static token"),
    }
}

const EXPECTED: &str = "\
fresnel-cli 0.1.0 doctor, 2026-10-13T08:00:00Z
environment: process in 5000.0 ms, 7 folders in PATH (4 in the PATH of this process)
  problem: the login shell /bin/zsh gave no environment in 5s
kubeconfig: 1 context
  file: /Users/tester/work.yaml
  no such file: /Users/tester/.kube/config
  problem: /Users/tester/work.yaml: the user `a` has an earlier entry, which wins

cluster ams (ams.example.test:443, plugin doctl)
  plugin        doctl in /opt/homebrew/bin, which only the login shell has in PATH; search 0.2 ms
  DNS           2.1 ms; 10.0.0.1:443, 10.0.0.2:443
  ALPN          HTTP/2: yes; HTTP/1.1: yes
  no hedge      20 connects, 1 held (5.0 %), 1 failed; TCP 11.0 / 12.5 / 30.2 ms; TLS 24.0 / 26.0 / 41.0 ms
  hedge         20 connects, 0 held (0.0 %); 36.0 / 39.5 / 351.0 ms; 21 attempts; delay 150 ms
  credential    the credential is valid; first token after 812.4 ms; 1 plugin run
  channels      each one live after 905.0 ms; 3 connections
  HTTP/2        stream limit of the server: 100
  ping          20.0 / 21.0 / 25.0 ms (small requests on the open connection)
                a later request failed: request channel: connection reset
  GET /version  v1.33.1 (linux/amd64)
  idle close    the idle connection ended after 90.4 s (the server closed the connection)
                the channels with a light request stayed open

cluster old (127.0.0.1:6443, plugin login)
  plugin        login not found; search 0.1 ms
  DNS           no lookup; 127.0.0.1:6443
  ALPN          HTTP/2: failed: the server did not select Http2; HTTP/1.1: yes
  credential    credential plugin `login`: stopped with exit status: 7; 2 plugin runs
                plugin: Error: no account
                plugin: Run `login init`.
  channels      no wait for an answer: the cluster has no token
                watches: pause after 3 failures; no connection after 1s and 2 attempts
  HTTP/2        stream limit of the server: not known
  ping          no time (small requests on the open connection)
  GET /version  failed: credential plugin `login`: stopped with exit status: 7

cluster token ()
  not measured  a static token

tasks of the app that ended:
  task credential [ams] failed: boom; restart in 100 ms
";

#[test]
fn the_text_report_has_one_line_for_each_measured_fact() {
    let report = Report {
        tool: "fresnel-cli 0.1.0".to_owned(),
        time: "2026-10-13T08:00:00Z".to_owned(),
        environment: Environment {
            source: "process".to_owned(),
            problem: Some("the login shell /bin/zsh gave no environment in 5s".to_owned()),
            resolve_ms: 5000.0,
            path_entries: 7,
            process_path_entries: 4,
        },
        kubeconfig: super::super::report::Kubeconfig {
            sources: vec!["/Users/tester/work.yaml".to_owned(), "/Users/tester/.kube/config".to_owned()],
            absent: vec!["/Users/tester/.kube/config".to_owned()],
            contexts: 1,
            problems: vec!["/Users/tester/work.yaml: the user `a` has an earlier entry, which wins".to_owned()],
        },
        clusters: vec![good_cluster(), bad_cluster(), unusable_cluster()],
        task_reports: vec!["task credential [ams] failed: boom; restart in 100 ms".to_owned()],
    };
    assert_eq!(render(&report), EXPECTED);
}

#[test]
fn a_cluster_with_no_answer_is_not_live_and_an_open_idle_connection_shows_its_wait() {
    let session = Session {
        credential: None,
        live_ms: None,
        channels: Vec::new(),
        stream_limit: None,
        ping_ms: None,
        ping_error: None,
        version: Step::failed("no answer on each channel in 20s"),
        idle_close: Some(IdleClose {
            waited_s: 120.0,
            closed_after_s: None,
            reason: None,
            light_request_kept_open: false,
        }),
    };
    let lines = session_lines(&session);
    assert_eq!(lines.first().map(String::as_str), Some("  channels      not live"));
    let idle = [
        "  idle close    the idle connection was open after 120.0 s",
        "                the channels with a light request did not stay open",
    ];
    assert_eq!(lines[lines.len() - 2..], idle);
}
