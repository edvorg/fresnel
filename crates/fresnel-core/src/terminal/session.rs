//! The state of the session of a terminal, and what a terminal of the dock is.

use std::sync::Arc;
use std::time::SystemTime;

use super::Terminal;
use crate::cluster::ClusterId;
use crate::store::ObjectKey;

/// The state of the stream between the terminal and its remote program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// A step before the stream: the new pod of a node shell or the new debug container,
    /// and the wait for its start.
    Starting { step: String },
    /// The request of the stream is on its way.
    Connecting,
    /// The remote program runs.
    Open,
    /// The input waits since this time: the program reads none, or the path is slow. The
    /// ping of the socket tells a break, so this state is no break.
    InputWaits { since: SystemTime },
    /// The app ends the session: the close of its tab, or of its cluster.
    Closing,
    /// The remote program ended with this exit code. The screen stays.
    Exited { code: i32 },
    /// The stream ended with no exit code and no fault, for example an attach that the
    /// server ended, or a session that the app closed.
    Ended { reason: String },
    /// The session ended with this error.
    Failed { reason: String },
}

impl SessionState {
    /// True for a state that does not change any more: the session and its clean-up ended.
    pub fn is_final(&self) -> bool {
        matches!(self, Self::Exited { .. } | Self::Ended { .. } | Self::Failed { .. })
    }
}

/// The program of a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Program {
    /// A shell in a container, started by an exec.
    Shell,
    /// The main process of a container, by an attach. A close leaves it as it is.
    Attach,
    /// A new debug container (an ephemeral container) with a shell, which sees the processes
    /// of the container of the request.
    Debug,
    /// A shell on a node, in a new privileged pod that the app deletes at the end.
    NodeShell,
}

impl Program {
    /// True for a program of the app, which the close of its tab ends. An attach reaches the
    /// main process of the container, which is not one.
    pub fn ends_with_tab(self) -> bool {
        !matches!(self, Self::Attach)
    }
}

/// What a terminal shows: a program in a container of a pod, or a shell on a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalRequest {
    pub cluster: ClusterId,
    /// The pod, or the node of a node shell, which has no namespace.
    pub object: ObjectKey,
    /// The UID of the object: a new pod or node of the same name is an other one.
    pub uid: Arc<str>,
    /// The container of a shell or an attach, the target of a debug container. Empty for a
    /// node shell.
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
