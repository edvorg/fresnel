//! Exec, attach and the tunnels of a port forward on the fake API server. Its pod has a
//! shell that sends each byte of its input back, and ports that do the same.
//!
//! - `exec`: the channels of exec and attach, the exit and the journal.
//! - `tunnel`: the port forward. `faults`: silent, reset and held connections, and a pod
//!   that reads nothing.

mod exec;
mod faults;
mod tunnel;

use std::time::Duration;

use fresnel_core::journal::Journal;
use fresnel_core::net::{NetSettings, SocketCounts, SocketSettings, UpgradeSettings, WallClock};
use fresnel_core::store::ObjectKey;
use fresnel_core::stream::{Exec, ExecSession, Output};

use crate::bed::Bed;
use crate::common::{in_time, until};

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
