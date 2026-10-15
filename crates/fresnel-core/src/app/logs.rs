//! The log views (plan section 12.7): one session for each view that the UI shows, in the
//! connected cluster of its object.
//!
//! A session is a part of the connected cluster, so it stops with the connections of that
//! cluster. A view that the UI shows again keeps its session and its lines; a view with new
//! options is a new request, so it gets a new session. A save of a view is a part of its
//! cluster too: it reads the lines of the session or the logs on the gateway.

use std::sync::Arc;

use super::{Inner, Phase};
use crate::cluster::{ClusterId, Gateway};
use crate::error::CoreError;
use crate::logs::{LogRequest, LogSave, LogSession, LogView, SaveParts, SaveView, Saves, SessionParts};
use crate::published::Held;

impl Inner {
    /// Shows these log views: each connected cluster runs the views of its objects and
    /// stops the others. A cluster that connects later starts its views then.
    pub(super) fn show_logs(&self, requests: Vec<LogRequest>) {
        let mut state = self.state();
        if state.phase == Phase::Stopped {
            return;
        }
        for connected in state.connected.values_mut() {
            connected.show_logs(&requests);
        }
        state.logs = requests;
        drop(state);
        self.repaint.request();
    }

    /// Starts a save of a log view in the cluster of its object.
    pub(super) fn save_log(&self, save: LogSave) {
        let mut state = self.state();
        match state.connected.get_mut(&save.request.cluster) {
            Some(connected) => connected.save_log(save),
            None => {
                let text = format!("cannot save the log: {} is not connected", save.request.cluster);
                self.journal.warning(super::APP, text);
            }
        }
        drop(state);
        self.repaint.request();
    }

    /// Stops a save of a log of a cluster.
    pub(super) fn cancel_save(&self, cluster: &ClusterId, number: u64) {
        if let Some(connected) = self.state().connected.get(cluster) {
            connected.cancel_save(number);
        }
    }
}

/// The log views of one connected cluster.
pub(super) struct ClusterLogs {
    cluster: ClusterId,
    sessions: Vec<LogSession>,
    saves: Saves,
    parts: SessionParts,
    gateway: Gateway,
}

impl ClusterLogs {
    pub(super) fn new(parts: SessionParts, gateway: Gateway, saves: SaveParts) -> Self {
        Self { cluster: gateway.cluster().clone(), sessions: Vec::new(), saves: Saves::new(saves), parts, gateway }
    }

    /// Runs a session for each request of this cluster, and stops each other session.
    pub(super) fn show(&mut self, requests: &[LogRequest]) {
        let own: Vec<&LogRequest> = requests.iter().filter(|request| request.cluster == self.cluster).collect();
        self.sessions.retain(|session| own.contains(&session.request()));
        for request in own {
            if !self.sessions.iter().any(|session| session.request() == request) {
                let (parts, gateway) = (self.parts.clone(), self.gateway.clone());
                self.sessions.push(LogSession::start(request.clone(), parts, gateway));
            }
        }
    }

    /// The views of the sessions, in the sequence of their start: a view of the UI finds its
    /// own by `LogView::request`.
    pub(super) fn views(&self) -> Arc<[Held<LogView>]> {
        self.sessions.iter().map(LogSession::view).collect()
    }

    /// Starts a save of the view of `save` as it is now. A view that the UI does not show
    /// has no session: its save fails with that reason.
    pub(super) fn save(&mut self, save: LogSave) {
        match self.sessions.iter().find(|session| session.request() == &save.request) {
            Some(session) => self.saves.start(save, &session.view(), &self.gateway),
            None => self.saves.refuse(save, CoreError::invalid("the log view is not open")),
        }
    }

    pub(super) fn cancel_save(&self, number: u64) {
        self.saves.cancel(number);
    }

    /// The saves, the oldest first.
    pub(super) fn saves(&self) -> Arc<[Held<SaveView>]> {
        self.saves.views()
    }
}
