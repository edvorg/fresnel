//! The state of the session of a terminal, and what a terminal of the dock is.

use std::sync::Arc;
use std::time::SystemTime;

use super::Terminal;
use crate::cluster::ClusterId;
use crate::store::ObjectKey;

/// The state of the stream between the terminal and its remote program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// The request of the session is on its way.
    Connecting,
    /// The remote program runs.
    Open,
    /// No data came since this time, also no answer to a ping: the connection can be lost.
    Quiet { since: SystemTime },
    /// The remote program ended with this exit code. The screen stays.
    Exited { code: i32 },
    /// The session ended with this error. The remote program can still run in the pod.
    Failed { reason: String },
}

/// The program of a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    /// A shell, started by an exec.
    Shell,
    /// The main process of the container, by an attach.
    Attach,
}

/// What a terminal shows: a program in a container of a pod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalRequest {
    pub cluster: ClusterId,
    pub pod: ObjectKey,
    /// The UID of the pod: a new pod of the same name is an other pod.
    pub uid: Arc<str>,
    pub container: Arc<str>,
    pub program: Program,
}

/// One terminal of the dock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalView {
    /// The identity of the terminal while the app runs.
    pub id: u64,
    pub request: TerminalRequest,
    pub terminal: Terminal,
}
