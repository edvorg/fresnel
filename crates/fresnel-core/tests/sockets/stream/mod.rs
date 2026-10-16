//! Exec, attach and the tunnels of a port forward on the fake API server. Its pod has a
//! shell that sends each byte of its input back, and ports that do the same.
//!
//! - `exec`: the channels of exec and attach, the exit and the journal.
//! - `tunnel`: the port forward. `faults`: silent, reset and held connections, and a pod
//!   that reads nothing. `cluster`: the signal and the channels of the cluster of a socket.

mod cluster;
mod exec;
mod faults;
mod tunnel;

use std::time::Duration;

use fresnel_core::cluster::Gateway;
use fresnel_core::journal::Journal;
use fresnel_core::net::{NetSettings, SocketCounts, SocketEnd, SocketSettings, UpgradeSettings, WallClock};
use fresnel_core::store::ObjectKey;
use fresnel_core::stream::{Exec, ExecEnd, ExecSession, Output};

use crate::bed::Bed;
use crate::common::{in_time, until};

const SECOND: Duration = Duration::from_secs(1);

/// The pod of the fake server.
fn pod() -> ObjectKey {
    ObjectKey::new("default", "web")
}

/// A shell with no terminal, which reads the input.
fn sh() -> Exec {
    Exec { pod: pod(), container: None, command: Some(vec!["sh".to_owned()]), tty: false, stdin: true }
}

fn journal() -> Journal {
    Journal::new(100, WallClock::system())
}

/// Each entry of the journal as `level source: text`.
fn entries(journal: &Journal) -> Vec<String> {
    let view = journal.view();
    view.entries.iter().map(|entry| format!("{:?} {}: {}", entry.level, entry.source, entry.text)).collect()
}

/// Short pings, so that a test of a silent connection takes 2 s.
fn short_pings() -> NetSettings {
    let socket = SocketSettings {
        ping_after: Duration::from_secs(1),
        answer_limit: Duration::from_secs(1),
        close_limit: Duration::from_secs(1),
        ..SocketSettings::default()
    };
    NetSettings { upgrade: UpgradeSettings { socket, ..UpgradeSettings::default() }, ..NetSettings::default() }
}

/// The output of a session to its end: the standard output and the error output.
async fn output(session: &mut ExecSession) -> (Vec<u8>, Vec<u8>) {
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    while let Some(output) = in_time("the output", session.next()).await {
        match output {
            Output::Stdout(data) => stdout.extend_from_slice(&data),
            Output::Stderr(data) => stderr.extend_from_slice(&data),
        }
    }
    (stdout, stderr)
}

/// The counters of the sockets of the cluster when no socket is open.
async fn closed_sockets(bed: &Bed) -> SocketCounts {
    until(&bed.link, "each socket ended", |status| status.upgrade.sockets.open == 0).await.upgrade.sockets
}

/// Opens a shell and proves that its bytes flow.
async fn flowing(gateway: &Gateway, journal: &Journal) -> ExecSession {
    let mut session = ExecSession::open(gateway, &sh(), journal.clone()).await.expect("an exec");
    session.input().send(b"echo ready\n").await.expect("the input");
    assert!(matches!(in_time("the echo", session.next()).await, Some(Output::Stdout(_))));
    session
}

/// The end of a session that read its output to the end.
fn socket_end(session: &ExecSession) -> SocketEnd {
    match session.end() {
        Some(ExecEnd::Socket(end)) => end,
        other => panic!("an end of the socket: {other:?}"),
    }
}
