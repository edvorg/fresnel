//! The terminals of the core on the fake API server (plan section 12.8), on each system: a
//! shell on the exec stream of the cluster of its request, and the end of a node shell whose
//! tab closes before its pod starts. The cluster test `k3s_terminal` runs real programs.

use fresnel_core::app::{Command, CoreSettings, CoreView, Health};
use fresnel_core::cluster::ClusterId;
use fresnel_core::journal::TASK_SOURCE;
use fresnel_core::store::ObjectKey;
use fresnel_core::terminal::{Frame, Palette, Program, SessionState, Terminal, TerminalRequest};
use fresnel_testkit::{FakeApiServer, SeenRequest};
use http::Method;

use super::{App, home, write_kubeconfig};
use crate::common::in_time;

const PODS: &str = "/api/v1/pods";
/// The pods of the namespace of the node shells in the default settings.
const NODE_SHELLS: &str = "/api/v1/namespaces/kube-system/pods";

/// The pod `web-1` with its container `app`, which runs.
fn pods() -> String {
    let pod = r#"{"metadata":{"name":"web-1","namespace":"shop","uid":"u1","resourceVersion":"9"},
        "spec":{"containers":[{"name":"app"}]},
        "status":{"phase":"Running","containerStatuses":[{"name":"app","image":"a","imageID":"","ready":true,
        "restartCount":0,"containerID":"containerd://a1","state":{"running":{}}}]}}"#;
    format!(r#"{{"kind":"PodList","apiVersion":"v1","metadata":{{"resourceVersion":"10"}},"items":[{pod}]}}"#)
}

/// The core with the cluster ams on the fake server, live, with the pods in its store.
async fn started(label: &str, ams: &FakeApiServer) -> App {
    ams.set_body(PODS, &pods());
    let home = home(label);
    write_kubeconfig(&home, "ams", &[("ams", ams.addr(), ams, false)]);
    let app = App::started(&home, CoreSettings::default());
    app.until_health("ams", Health::Live).await;
    let listed = |view: &CoreView| {
        let row = view.clusters.iter().find(|row| row.id.as_str() == "ams");
        row.and_then(|row| row.data.as_ref()).is_some_and(|data| data.pods.listed)
    };
    app.until("the pods of ams", listed).await;
    app
}

fn open(app: &App, object: ObjectKey, uid: &str, container: &str, program: Program) {
    let (cluster, uid, container) = (ClusterId::new("ams"), uid.into(), container.into());
    app.core.send(Command::OpenTerminal(TerminalRequest { cluster, object, uid, container, program }));
}

/// The one terminal of the view.
async fn terminal(app: &App) -> (u64, Terminal) {
    let view = app.until("a terminal", |view| !view.terminals.is_empty()).await;
    let first = view.terminals.first().expect("a terminal");
    (first.id, first.terminal.clone())
}

fn screen(terminal: &Terminal) -> String {
    let mut frame = Frame::default();
    terminal.frame(&mut frame, &Palette::DARK);
    (0..usize::from(frame.size.rows)).map(|row| frame.text(row)).collect::<Vec<_>>().join("\n")
}

/// The exec requests that the server saw.
fn execs(server: &FakeApiServer) -> Vec<SeenRequest> {
    server.requests().into_iter().filter(|seen| seen.uri.path().ends_with("/exec")).collect()
}

/// A shell is an exec of the launch with the mark of its session and a terminal; the size
/// goes first (the fake pod writes it back), each key goes to the program, and the exit code
/// of the process decides the end. A shell that ended itself gets no second exec.
#[tokio::test]
async fn a_shell_runs_on_the_exec_stream_of_its_cluster_and_its_exit_code_ends_it() {
    let ams = FakeApiServer::start().await.expect("a server");
    let mut app = started("terminal-shell", &ams).await;
    open(&app, ObjectKey::new("shop", "web-1"), "u1", "app", Program::Shell);
    let (_, terminal) = terminal(&app).await;
    app.until("the size on the screen", |_| screen(&terminal).contains(r#"size {"Width":80,"Height":24}"#)).await;
    assert_eq!(terminal.state(), SessionState::Open);
    let exec = execs(&ams).first().map(|seen| seen.uri.to_string()).expect("the exec");
    assert!(exec.starts_with("/api/v1/namespaces/shop/pods/web-1/exec?"), "{exec}");
    for part in ["stdin=true", "tty=true", "container=app", "command=sh", "FRESNEL_TERMINAL"] {
        assert!(exec.contains(part), "{part} in {exec}");
    }
    terminal.send(b"echo typed\nexit 7\n").expect("room");
    app.until("the exit code", |_| terminal.state() == SessionState::Exited { code: 7 }).await;
    assert!(screen(&terminal).contains("echo typed"), "the keys reached the program: {}", screen(&terminal));
    app.until_texts(TASK_SOURCE, |ended| ended.len() == 1).await;
    app.expected_task_ends = 1;
    assert_eq!(execs(&ams).len(), 1, "no second exec");
}

/// A close of the tab while the pod of a node shell waits for its start deletes the pod at
/// once: the wait for the start of the pod, up to 3 minutes, does not hold a privileged pod.
#[tokio::test]
async fn a_close_before_the_start_of_a_node_shell_deletes_its_pod_at_once() {
    let ams = FakeApiServer::start().await.expect("a server");
    let created =
        r#"{"apiVersion":"v1","kind":"Pod","metadata":{"name":"made","namespace":"kube-system","uid":"u-made"}}"#;
    ams.set_body(NODE_SHELLS, created);
    let mut app = started("terminal-node", &ams).await;
    open(&app, ObjectKey::new("", "n1"), "node-uid", "", Program::NodeShell);
    let (id, terminal) = terminal(&app).await;
    let waits =
        |state: &SessionState| matches!(state, SessionState::Starting { step } if step.contains("not in the store"));
    app.until("the wait for the start of the pod", |_| waits(&terminal.state())).await;
    app.core.send(Command::CloseTerminal(id));
    let deleted = |seen: &SeenRequest| {
        seen.method == Method::DELETE && seen.uri.path().starts_with(&format!("{NODE_SHELLS}/fresnel-shell-n1-"))
    };
    in_time("the delete of the pod", ams.wait_for(|_| ams.requests().iter().any(deleted))).await;
    let texts = app.until_texts("ams", |texts| texts.iter().any(|text| text.ends_with(" is deleted"))).await;
    assert!(
        texts.iter().any(|text| text.starts_with("the node shell pod kube-system/fresnel-shell-n1-")),
        "{texts:#?}"
    );
    app.until_texts(TASK_SOURCE, |ended| ended.len() == 1).await;
    app.expected_task_ends = 1;
    assert_eq!(terminal.state(), SessionState::Ended { reason: "the app closed the session before its start".into() });
}
