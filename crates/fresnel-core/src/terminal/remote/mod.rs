//! The session of a terminal (plan section 12.8): one task that connects a [`Terminal`] with
//! its remote program through an exec or an attach on a socket of the gateway.
//!
//! The socket proves the connection with its ping; the line of the tab shows the state of the
//! session, and the status of the process decides the final state. A close of the owner ends
//! a program of the app: Ctrl-D, then a hangup through a second exec, and the delete of the
//! pod of a node shell. A stream that broke leaves its shell in the pod; the close of its tab
//! then ends it.
//!
//! - `prepare`: the pod in its store, a new pod of a node shell or a new debug container.
//! - `pump`: the bytes of an open stream. `end`: the close, the signal, the final state.
//! - `script`: the start of a shell with a mark, and the end of each process with it.

mod end;
mod prepare;
mod pump;
mod script;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use fresnel_net::task::{Restart, TaskHandle, TaskName, Tasks};
use k8s_openapi::api::core::v1::Pod;
use tokio::sync::watch;

use crate::cluster::Gateway;
use crate::error::CoreError;
use crate::journal::Journal;
use crate::ops::Target;
use crate::published::Reader;
use crate::store::Snapshot;
use crate::stream::{Exec, ExecEnd, ExecSession};
use crate::terminal::{Program, SessionState, Terminal, TerminalRequest};
use pump::{Pumped, Streams};
use script::Mark;

/// Asks the UI for a repaint.
pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

/// The settings of the start of a session, from the settings file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Launch {
    /// The shells to try, in this sequence.
    pub(crate) shells: Vec<String>,
    pub(crate) debug_image: String,
    pub(crate) node_image: String,
    /// The namespace of the pod of a node shell.
    pub(crate) node_namespace: String,
}

/// What a session needs of its cluster and of the app.
#[derive(Clone)]
pub(crate) struct Parts {
    /// The gateway of the cluster. A session holds it to its end, so the second exec and the
    /// delete of a pod still go after the user removed the cluster from the selection.
    pub(crate) gateway: Gateway,
    /// The pods of the cluster, for the state of a pod and of a new container.
    pub(crate) pods: Reader<Snapshot<Pod>>,
    pub(crate) journal: Journal,
    pub(crate) wake: Wake,
    pub(crate) launch: Launch,
}

/// One session. Its task stops when this value drops: [`RemoteSession::close`] first ends
/// the remote program, and [`RemoteSession::is_done`] tells the end of that work.
pub(crate) struct RemoteSession {
    close: watch::Sender<bool>,
    done: Arc<AtomicBool>,
    _task: TaskHandle,
}

impl RemoteSession {
    /// Starts the session of a terminal. The terminal shows its state.
    pub(crate) fn start(tasks: &Tasks, request: TerminalRequest, terminal: Terminal, parts: Parts) -> Self {
        let (close, closed) = watch::channel(false);
        let done = Arc::new(AtomicBool::new(false));
        let name = TaskName::scoped("terminal session", request.cluster.as_str());
        let mut once = Some((request, terminal, parts, closed, Arc::clone(&done)));
        let task = tasks.start(name, Restart::Never, move || {
            let once = once.take();
            async move {
                match once {
                    Some((request, terminal, parts, closed, done)) => {
                        let _done = Done { terminal: terminal.clone(), done, wake: Arc::clone(&parts.wake) };
                        run(&request, &terminal, &parts, closed).await;
                        Ok(())
                    }
                    None => Err(CoreError::invalid("the task of a terminal session ran a second time")),
                }
            }
        });
        Self { close, done, _task: task }
    }

    /// Ends the session. The task ends the remote program and then sets the final state.
    pub(crate) fn close(&self) {
        self.close.send_replace(true);
    }

    /// True after the end of the task, with its clean-up.
    pub(crate) fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
}

/// The end of the task, also by a panic: a terminal with no final state then fails, and
/// the journal has the report of the task.
struct Done {
    terminal: Terminal,
    done: Arc<AtomicBool>,
    wake: Wake,
}

impl Drop for Done {
    fn drop(&mut self) {
        if !self.terminal.state().is_final() {
            let reason = "the task of the session stopped; the journal tells why".to_owned();
            self.terminal.set_state(SessionState::Failed { reason });
        }
        self.done.store(true, Ordering::Release);
        (self.wake)();
    }
}

/// The work of a session to its end.
async fn run(request: &TerminalRequest, terminal: &Terminal, parts: &Parts, mut close: watch::Receiver<bool>) {
    let mark = Mark::new();
    let mut own_pod = None;
    let state = match open(request, (terminal, parts), &mark, &mut own_pod).await {
        Ok(mut opened) => talk(request, (terminal, parts), &mut opened, (&mark, &mut close)).await,
        Err(error) => SessionState::Failed { reason: error.to_string() },
    };
    if let Some(pod) = own_pod {
        delete(parts, &pod).await;
    }
    terminal.set_state(state);
}

/// An open stream.
struct Opened {
    stream: ExecSession,
    streams: Streams,
    /// The container of the stream: the one of the request, or a new one of the app.
    container: String,
}

/// Prepares the place of the stream and opens it. The pod of a node shell goes into `own_pod`
/// as soon as it exists, so that each later failure deletes it.
async fn open(
    request: &TerminalRequest,
    (terminal, parts): (&Terminal, &Parts),
    mark: &Mark,
    own_pod: &mut Option<Target>,
) -> Result<Opened, CoreError> {
    let shells = &parts.launch.shells;
    let both = Streams { tty: true, stdin: true };
    let (exec, streams) = match request.program {
        Program::Shell => {
            prepare::pod_of(parts, request)?;
            let command = Some(script::launch(shells, mark));
            (exec(&request.object, &request.container, command, both), both)
        }
        Program::Attach => {
            let pod = prepare::pod_of(parts, request)?;
            // A pod that the store does not know yet gets each stream; the server decides.
            let flags = pod.and_then(|pod| prepare::spec_flags(&pod, &request.container)).unwrap_or((true, true));
            let streams = Streams { stdin: flags.0, tty: flags.1 };
            if !streams.stdin {
                // The keys would go nowhere: the terminal refuses them, and the tab tells why.
                terminal.refuse_input();
            }
            (exec(&request.object, &request.container, None, streams), streams)
        }
        Program::Debug => {
            prepare::pod_of(parts, request)?;
            let name = crate::ops::shell::debug_name();
            prepare::add_debug(parts, (request, terminal), &name, &script::launch(shells, mark)).await?;
            (exec(&request.object, &name, None, both), both)
        }
        Program::NodeShell => {
            let pod = prepare::create_node_pod(parts, (request, terminal), &script::on_node(shells, mark)).await?;
            let what = format!("the pod {}/{}", pod.key.namespace, pod.key.name);
            let key = pod.key.clone();
            let uid = Arc::clone(&pod.uid);
            *own_pod = Some(pod);
            prepare::until_running(parts, terminal, (&key, &uid), "shell", &what).await?;
            (exec(&key, "shell", None, both), both)
        }
    };
    terminal.set_state(SessionState::Connecting);
    (parts.wake)();
    let stream = ExecSession::open(&parts.gateway, &exec, parts.journal.clone()).await?;
    // A new container of the app printed its prompt before the attach: a line end shows a
    // new one. The input is empty at the start, so the line end fits.
    if matches!(request.program, Program::Debug | Program::NodeShell) {
        terminal.send(b"\r").map_err(|refused| CoreError::invalid(refused.to_string()))?;
    }
    Ok(Opened { stream, streams, container: exec.container.unwrap_or_default() })
}

fn exec(pod: &crate::store::ObjectKey, container: &str, command: Option<Vec<String>>, streams: Streams) -> Exec {
    Exec { pod: pod.clone(), container: Some(container.to_owned()), command, tty: streams.tty, stdin: streams.stdin }
}

/// Runs an open stream to its end, ends the program of the app after a close, and gives the
/// final state. A stream that broke while its shell ran waits for the close of its tab,
/// whose signal then ends the shell.
async fn talk(
    request: &TerminalRequest,
    (terminal, parts): (&Terminal, &Parts),
    opened: &mut Opened,
    (mark, close): (&Mark, &mut watch::Receiver<bool>),
) -> SessionState {
    let (stream, streams, wake) = (&mut opened.stream, opened.streams, &parts.wake);
    let shell = request.program.ends_with_tab();
    let closed = pump::pump(stream, terminal, (streams, wake), close).await == Pumped::Close;
    if closed {
        end::close(stream, terminal, (streams, wake), shell).await;
    }
    let ended = stream.end();
    let exited = matches!(ended, Some(ExecEnd::Exited(_)));
    let state = end::final_state(ended, closed);
    // A shell of the app in a container that goes on after it: a shell of the request, or
    // a debug container whose shell did not end. A node shell goes with the delete of its pod.
    let signalled = match request.program {
        Program::Shell => true,
        Program::Debug => !exited,
        Program::Attach | Program::NodeShell => false,
    };
    if !signalled {
        return state;
    }
    if !closed {
        if exited {
            return state;
        }
        let note = "the shell can still run in the pod: a close of the tab ends it";
        terminal.set_state(with_note(&state, note));
        (parts.wake)();
        while !*close.borrow_and_update() {
            if close.changed().await.is_err() {
                return state;
            }
        }
    }
    let what = format!("the shell in pod {}/{}", request.object.namespace, request.object.name);
    let source = request.cluster.as_str();
    match end::signal(parts, &request.object, &opened.container, mark).await {
        Ok(count) => parts.journal.info(source, format!("{what}: a hangup went to {count} processes")),
        Err(error) => {
            parts.journal.warning(source, format!("{what} can still run: the signal did not go: {error}"));
            return with_note(&state, &format!("the shell can still run in the pod: the signal did not go: {error}"));
        }
    }
    state
}

/// A state with a note on what can still run in the pod.
fn with_note(state: &SessionState, note: &str) -> SessionState {
    match state {
        SessionState::Failed { reason } => SessionState::Failed { reason: format!("{reason}; {note}") },
        SessionState::Ended { reason } => SessionState::Ended { reason: format!("{reason}; {note}") },
        other => other.clone(),
    }
}

/// Deletes the pod of a node shell, with a journal entry for a pod that stays.
async fn delete(parts: &Parts, pod: &Target) {
    let what = format!("the node shell pod {}/{}", pod.key.namespace, pod.key.name);
    match end::delete_pod(parts, pod).await {
        Ok(()) => parts.journal.info(pod.cluster.as_str(), format!("{what} is deleted")),
        Err(error) => parts.journal.warning(pod.cluster.as_str(), format!("{what} stays, delete it: {error}")),
    }
}
