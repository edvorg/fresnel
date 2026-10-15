//! The log views (plan section 12.7): one session for each view that the UI shows, in the
//! connected cluster of its object.
//!
//! A session is a part of the connected cluster, so it stops with the connections of that
//! cluster. A view that the UI shows again keeps its session and its lines; a view with new
//! options is a new request, so it gets a new session.

use std::sync::Arc;

use super::{Inner, Phase};
use crate::cluster::{ClusterId, Gateway};
use crate::logs::{LogRequest, LogSession, LogView, SessionParts};
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
}

/// The log views of one connected cluster.
pub(super) struct ClusterLogs {
    cluster: ClusterId,
    sessions: Vec<LogSession>,
    parts: SessionParts,
    gateway: Gateway,
}

impl ClusterLogs {
    pub(super) fn new(parts: SessionParts, gateway: Gateway) -> Self {
        Self { cluster: gateway.cluster().clone(), sessions: Vec::new(), parts, gateway }
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

    /// The views of the sessions, in the sequence of their requests.
    pub(super) fn views(&self) -> Arc<[Held<LogView>]> {
        self.sessions.iter().map(LogSession::view).collect()
    }
}
