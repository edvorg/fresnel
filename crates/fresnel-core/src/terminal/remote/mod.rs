//! The session of a terminal (plan section 12.8): one task that connects a [`Terminal`] with
//! its remote program through an exec or an attach on a socket of the gateway.
//!
//! The socket proves the connection with its ping; the line of the tab shows the state of the
//! session, and the status of the process decides the final state. A close of the owner ends
//! a program of the app: Ctrl-D, then a hangup through a second exec, and the delete of the
//! pod of a node shell. A stream that broke, and a debug container that no stream reached,
//! leave their shell in the pod; the close of the tab then ends it.
//!
//! - `prepare`: the pod in its store, a new pod of a node shell or a new debug container.
//! - `pump`: the bytes of an open stream. `end`: the close, the signal, the final state.
//! - `script`: the start of a shell with a mark, and the end of each process with it.

mod end;
mod prepare;
mod pump;
mod script;

use std::sync::Arc;

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
/// the remote program, and [`RemoteSession::ended`] waits for the end of that work.
pub(crate) struct RemoteSession {
    close: watch::Sender<bool>,
    done: watch::Receiver<bool>,
    _task: TaskHandle,
}

impl RemoteSession {
    /// Starts the session of a terminal. The terminal shows its state.
    pub(crate) fn start(tasks: &Tasks, request: TerminalRequest, terminal: Terminal, parts: Parts) -> Self {
        let (close, closed) = watch::channel(false);
        let (finished, done) = watch::channel(false);
        let name = TaskName::scoped("terminal session", request.cluster.as_str());
        let mut once = Some((request, terminal, parts, closed, finished));
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
        *self.done.borrow()
    }

    /// Waits for the end of the task, with its clean-up. A task that dropped before its run
    /// has no work left, so its end counts too.
    pub(crate) fn ended(&self) -> impl Future<Output = ()> + use<> {
        let mut done = self.done.clone();
        async move {
            while !*done.borrow_and_update() {
                if done.changed().await.is_err() {
                    break;
                }
            }
        }
    }
}

/// The end of the task, also by a panic: a terminal with no final state then fails, and
/// the journal has the report of the task.
struct Done {
    terminal: Terminal,
    done: watch::Sender<bool>,
    wake: Wake,
}

impl Drop for Done {
    fn drop(&mut self) {
        if !self.terminal.state().is_final() {
            let reason = "the task of the session stopped; the journal tells why".to_owned();
            self.terminal.set_state(SessionState::Failed { reason });
        }
        self.done.send_replace(true);
        (self.wake)();
    }
}

/// What the session made in the cluster, for its end.
#[derive(Default)]
struct Made {
    /// The pod of a node shell: the end of the session deletes it.
    pod: Option<Target>,
    /// A new debug container that runs: its shell waits for input until a stream ends it.
    debug: Option<String>,
}

/// The work of a session to its end.
async fn run(request: &TerminalRequest, terminal: &Terminal, parts: &Parts, mut close: watch::Receiver<bool>) {
    let mark = Mark::new();
    let mut made = Made::default();
    let state = match open(request, (terminal, parts), (&mark, &mut close), &mut made).await {
        Ok(Some(mut opened)) => talk(request, (terminal, parts), &mut opened, (&mark, &mut close)).await,
        Ok(None) => SessionState::Ended { reason: "the app closed the session before its start".to_owned() },
        Err(error) => {
            let state = SessionState::Failed { reason: error.to_string() };
            match &made.debug {
                // The attach failed, and the shell of the debug container waits in the pod.
                Some(container) => {
                    let place = (container.as_str(), &mark);
                    hang_up(request, (terminal, parts), place, (&mut close, false), state).await
                }
                None => state,
            }
        }
    };
    if let Some(pod) = made.pod {
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

/// Prepares the place of the stream and opens it. `None` for a close before the start of a
/// node shell. What the session makes goes into `made` as soon as it exists, so that each
/// later failure ends it.
async fn open(
    request: &TerminalRequest,
    (terminal, parts): (&Terminal, &Parts),
    (mark, close): (&Mark, &mut watch::Receiver<bool>),
    made: &mut Made,
) -> Result<Option<Opened>, CoreError> {
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
            made.debug = Some(name.clone());
            (exec(&request.object, &name, None, both), both)
        }
        Program::NodeShell => {
            let pod = prepare::create_node_pod(parts, (request, terminal), &script::on_node(shells, mark)).await?;
            let what = format!("the pod {}/{}", pod.key.namespace, pod.key.name);
            let (key, uid) = (pod.key.clone(), Arc::clone(&pod.uid));
            made.pod = Some(pod);
            // A close ends the wait for the start, as the delete of the pod ends its shell. It
            // does not cut the create: a pod whose answer it cut would stay.
            tokio::select! {
                started = prepare::until_running(parts, terminal, (&key, &uid), "shell", &what) => started?,
                () = until_closed(close) => return Ok(None),
            }
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
    Ok(Some(Opened { stream, streams, container: exec.container.unwrap_or_default() }))
}

fn exec(pod: &crate::store::ObjectKey, container: &str, command: Option<Vec<String>>, streams: Streams) -> Exec {
    Exec { pod: pod.clone(), container: Some(container.to_owned()), command, tty: streams.tty, stdin: streams.stdin }
}

/// Runs an open stream to its end, ends the program of the app after a close, and gives the
/// final state.
async fn talk(
    request: &TerminalRequest,
    (terminal, parts): (&Terminal, &Parts),
    opened: &mut Opened,
    (mark, close): (&Mark, &mut watch::Receiver<bool>),
) -> SessionState {
    let (stream, streams, wake) = (&mut opened.stream, opened.streams, &parts.wake);
    let closed = pump::pump(stream, terminal, (streams, wake), close).await == Pumped::Close;
    if closed {
        end::close(stream, terminal, (streams, wake), request.program.ends_with_tab()).await;
    }
    let ended = stream.end();
    let exited = matches!(ended, Some(ExecEnd::Exited(_)));
    let state = end::final_state(ended, closed);
    if leaves_shell(request.program, closed, exited) {
        let place = (opened.container.as_str(), mark);
        return hang_up(request, (terminal, parts), place, (close, closed), state).await;
    }
    state
}

/// True when a shell of the app can still run in its container after the stream: a shell of
/// the request after a close (its jobs too) or with no exit, and a debug container whose
/// shell did not exit. A node shell goes with its pod; an attach reaches no program of the app.
fn leaves_shell(program: Program, closed: bool, exited: bool) -> bool {
    match program {
        Program::Shell => closed || !exited,
        Program::Debug => !exited,
        Program::Attach | Program::NodeShell => false,
    }
}

/// Ends a shell of the app in `container` with a hangup through a second exec. With no close
/// of the owner yet (a broken stream, a failed attach), the shell can still be of use: the
/// line tells it, and the hangup waits for the close of the tab.
async fn hang_up(
    request: &TerminalRequest,
    (terminal, parts): (&Terminal, &Parts),
    (container, mark): (&str, &Mark),
    (close, closed): (&mut watch::Receiver<bool>, bool),
    state: SessionState,
) -> SessionState {
    if !closed {
        terminal.set_state(with_note(&state, "the shell can still run in the pod: a close of the tab ends it"));
        (parts.wake)();
        until_closed(close).await;
    }
    let what = format!("the shell in pod {}/{}", request.object.namespace, request.object.name);
    let source = request.cluster.as_str();
    match end::signal(parts, &request.object, container, mark).await {
        Ok(count) => {
            parts.journal.info(source, format!("{what}: a hangup went to {count} processes"));
            state
        }
        Err(error) => {
            parts.journal.warning(source, format!("{what} can still run: the signal did not go: {error}"));
            with_note(&state, &format!("the shell can still run in the pod: the signal did not go: {error}"))
        }
    }
}

/// Waits for the close of the owner. An owner that dropped stops the task, so the wait goes on.
async fn until_closed(close: &mut watch::Receiver<bool>) {
    if close.wait_for(|closed| *closed).await.is_err() {
        std::future::pending::<()>().await;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The hangup goes where a shell of the app can still run (plan section 12.8: a closed tab
    /// leaves no process): after each close of a shell, which can have jobs, and after each
    /// end of a shell or a debug container with no exit status.
    #[test]
    fn a_hangup_goes_where_a_shell_of_the_app_can_still_run() {
        let each = |program| {
            [(true, true), (true, false), (false, true), (false, false)].map(|(c, e)| leaves_shell(program, c, e))
        };
        assert_eq!(each(Program::Shell), [true, true, false, true], "closed or with no exit");
        assert_eq!(each(Program::Debug), [false, true, false, true], "its container ends with its shell");
        assert_eq!(each(Program::Attach), [false; 4], "the main process is no program of the app");
        assert_eq!(each(Program::NodeShell), [false; 4], "the delete of its pod ends it");
    }
}
