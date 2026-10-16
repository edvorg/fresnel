//! The terminals of the dock (plan section 12.8): the core holds each one with its session,
//! the view gets a copy of the list, and a close removes one.
//!
//! A terminal stays after the end of its session, with its screen and the final state, until
//! the user closes it: the end of a shell is no reason to lose its output. A closed tab leaves
//! the list at once; its session ends its remote program in the background and drops after
//! that work. A cluster that leaves the selection closes the sessions of its terminals, and
//! their tabs stay with the final state.

use std::sync::Arc;

use super::{Inner, Phase};
use crate::cluster::ClusterId;
use crate::config::Config;
use crate::config::table::Setting;
use crate::terminal::{
    Launch, Options, Program, RemoteSession, SessionParts, SessionState, Terminal, TerminalRequest, TerminalView,
};

/// The terminals and their sessions.
#[derive(Default)]
pub(super) struct Terminals {
    last: u64,
    shown: Vec<(TerminalView, Option<RemoteSession>)>,
    /// The sessions of closed tabs while they end their remote program.
    closing: Vec<RemoteSession>,
    views: Arc<[TerminalView]>,
}

impl Terminals {
    /// The terminals of the dock, in the sequence of their open.
    pub(super) fn views(&self) -> Arc<[TerminalView]> {
        Arc::clone(&self.views)
    }

    /// Takes each session, for the stop of the core.
    pub(super) fn stop(&mut self) -> Vec<RemoteSession> {
        let shown = self.shown.iter_mut().filter_map(|(_, session)| session.take());
        shown.chain(self.closing.drain(..)).collect()
    }

    /// Closes the sessions of the terminals of these clusters. Their tabs stay.
    pub(super) fn close_sessions_of(&self, clusters: &[ClusterId]) {
        let of = self.shown.iter().filter(|(view, _)| clusters.contains(&view.request.cluster));
        for session in of.filter_map(|(_, session)| session.as_ref()) {
            session.close();
        }
    }

    /// Adds a terminal. Gives the sessions of closed tabs that ended, for a drop after the lock.
    fn add(
        &mut self,
        request: TerminalRequest,
        terminal: Terminal,
        session: Option<RemoteSession>,
    ) -> Vec<RemoteSession> {
        self.last += 1;
        self.shown.push((TerminalView { id: self.last, request, terminal }, session));
        self.publish()
    }

    /// Removes the terminal `id` and closes its session, which goes on until its end.
    fn close(&mut self, id: u64) -> Vec<RemoteSession> {
        let (closed, kept): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.shown).into_iter().partition(|(view, _)| view.id == id);
        self.shown = kept;
        for session in closed.into_iter().filter_map(|(_, session)| session) {
            session.close();
            self.closing.push(session);
        }
        self.publish()
    }

    /// Makes the copy for the view, and takes the sessions of closed tabs that ended.
    fn publish(&mut self) -> Vec<RemoteSession> {
        self.views = self.shown.iter().map(|(view, _)| view.clone()).collect();
        let (ended, going): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.closing).into_iter().partition(RemoteSession::is_done);
        self.closing = going;
        ended
    }
}

impl Inner {
    /// Opens a terminal and starts its session in the cluster of its request. A cluster that
    /// is not connected, and a change of a read-only cluster, give a terminal that failed
    /// with the reason: the dock shows it.
    pub(super) fn open_terminal(&self, request: TerminalRequest) {
        let mut state = self.state();
        if state.phase == Phase::Stopped {
            return;
        }
        let config = Arc::clone(&state.config);
        let scrollback = usize::try_from(config.number(Setting::TerminalScrollback)).unwrap_or(usize::MAX);
        let terminal = Terminal::new(Options { scrollback, ..Options::default() });
        let writes = matches!(request.program, Program::Debug | Program::NodeShell);
        let refusal = config.refusal(&request.cluster).filter(|_| writes);
        let session = match (refusal, state.connected.get(&request.cluster)) {
            (Some(reason), _) => {
                terminal.set_state(SessionState::Failed { reason });
                None
            }
            (None, None) => {
                let reason = format!("{} is not connected", request.cluster);
                terminal.set_state(SessionState::Failed { reason });
                None
            }
            (None, Some(connected)) => {
                let (gateway, pods) = connected.terminal_parts();
                let repaint = self.repaint.clone();
                let wake = Arc::new(move || repaint.request());
                let parts =
                    SessionParts { gateway, pods, journal: self.journal.clone(), wake, launch: launch(&config) };
                Some(RemoteSession::start(&self.tasks, request.clone(), terminal.clone(), parts))
            }
        };
        let ended = state.terminals.add(request, terminal, session);
        drop(state);
        drop(ended);
        self.repaint.request();
    }

    /// Removes a terminal of the dock. Its session ends its remote program.
    pub(super) fn close_terminal(&self, id: u64) {
        let ended = self.state().terminals.close(id);
        // The sessions drop after the lock.
        drop(ended);
        self.repaint.request();
    }

    /// Gives each terminal the lines of history of the settings: they act at once.
    pub(super) fn follow_terminal_settings(&self, config: &Config) {
        let lines = usize::try_from(config.number(Setting::TerminalScrollback)).unwrap_or(usize::MAX);
        let terminals = self.state().terminals.views();
        for view in terminals.iter() {
            view.terminal.set_scrollback(lines);
        }
    }
}

/// The settings of the start of a session.
fn launch(config: &Config) -> Launch {
    Launch {
        shells: config.list(Setting::TerminalShells),
        debug_image: config.text(Setting::DebugImage).to_owned(),
        node_image: config.text(Setting::NodeShellImage).to_owned(),
        node_namespace: config.text(Setting::NodeShellNamespace).to_owned(),
    }
}

#[cfg(test)]
mod tests;
