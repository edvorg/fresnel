//! The copy of the state of the app that the UI reads (plan section 7.3).
//!
//! Each value is a copy. The UI never holds a lock that a task needs.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use fresnel_net::ClusterStatus;

use super::data::ClusterData;
use super::health::{self, Health};
use super::manifest::ManifestRead;
use crate::cluster::{ClusterId, ClusterView, LinkState};
use crate::error::CoreError;
use crate::journal::JournalView;
use crate::kubeconfig::AuthKind;
use crate::state_file::TableLayout;

/// How far the start of the core is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Startup {
    /// The core did not start yet. It did no work of any kind.
    Waiting,
    /// The login shell gives its environment.
    Environment,
    /// The kubeconfig files load.
    Kubeconfig,
    /// The list of the clusters is complete.
    Ready,
}

/// One cluster of the kubeconfig files with the state of its connections.
#[derive(Clone, Debug)]
pub struct ClusterRow {
    /// The identity of the cluster.
    pub id: ClusterId,
    /// The host and the port of its server, or why the app cannot use its kubeconfig entry.
    pub server: Result<String, CoreError>,
    /// How its user proves the identity. `None` for an entry that the app cannot use.
    pub auth: Option<AuthKind>,
    /// True when the user selected the cluster. The app connects each selected cluster.
    pub selected: bool,
    /// The state of its connections.
    pub link: LinkState,
    /// The objects of the cluster with the states of their stores. `None` for a cluster
    /// that is not connected.
    pub data: Option<ClusterData>,
}

impl ClusterRow {
    pub(super) fn new(view: ClusterView, selected: bool, data: Option<ClusterData>) -> Self {
        let access = view.context.access.as_ref();
        Self {
            server: access.map(|access| access.server.to_string()).map_err(Clone::clone),
            auth: access.ok().map(|access| access.auth()),
            id: view.id,
            selected,
            link: view.state,
            data,
        }
    }

    /// The one state of the cluster.
    pub fn health(&self) -> Health {
        Health::of(&self.link)
    }

    /// Why the cluster is not live. The text is safe for a log.
    pub fn reason(&self) -> Option<String> {
        match &self.link {
            LinkState::Off => None,
            LinkState::Failed(error) => Some(error.to_string()),
            LinkState::On(status) => health::reason(status),
        }
    }

    /// The status of a connected cluster.
    pub fn status(&self) -> Option<&ClusterStatus> {
        match &self.link {
            LinkState::On(status) => Some(status),
            LinkState::Off | LinkState::Failed(_) => None,
        }
    }

    /// The time since the last answer of the server on a channel. `None` with no answer yet.
    pub fn last_answer_age(&self) -> Option<Duration> {
        let status = self.status()?;
        let groups = [&status.watches, &status.requests, &status.logs];
        groups.iter().flat_map(|group| &group.channels).filter_map(|channel| channel.last_frame_age).min()
    }
}

/// The login shell gave no environment. The app then finds a credential plugin only in
/// its own `PATH` and in the fixed folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellNote {
    /// Why. The text is safe for a log.
    pub problem: String,
    /// The last text that the profile of the shell wrote. Show it in the UI only: a
    /// profile can print a secret.
    pub detail: String,
}

/// The state of the app at one moment.
#[derive(Clone, Debug)]
pub struct CoreView {
    /// How far the start is.
    pub startup: Startup,
    /// Set when the login shell gave no environment.
    pub shell_problem: Option<ShellNote>,
    /// Each cluster of the kubeconfig files, in the sequence of the files.
    pub clusters: Vec<ClusterRow>,
    /// The layout of each table that the user changed, by the name of the table: from the
    /// state file and from the commands of this run. `None` before the core read the file.
    pub tables: Option<Arc<BTreeMap<String, TableLayout>>>,
    /// The last read of a manifest that a view asked for.
    pub manifest: Option<Arc<ManifestRead>>,
    /// The journal.
    pub journal: Arc<JournalView>,
}
