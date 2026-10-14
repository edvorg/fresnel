//! What the UI asks the core to do.

use std::collections::BTreeSet;

use super::manifest::ManifestRequest;
use crate::cluster::ClusterId;
use crate::keys::{NewSecret, Save, Target};
use crate::kinds::Kind;
use crate::state_file::TableLayout;

/// What the UI asks the core to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Selects a cluster or removes it from the selection. The core connects each
    /// selected cluster.
    Select {
        /// The cluster.
        cluster: ClusterId,
        /// True to select it.
        selected: bool,
    },
    /// "Retry now" for one cluster: each retry delay ends, that of a connect one time in 10 s
    /// at most, and a failed plugin runs again. For a cluster whose connect did not start,
    /// the kubeconfig files load again.
    RetryNow(ClusterId),
    /// Saves the layout of a table that the user changed. An empty layout removes the table
    /// from the state file.
    SaveTable {
        /// The name of the table.
        table: String,
        /// What the user changed of it.
        layout: TableLayout,
    },
    /// The views show the objects of these kinds: the core holds a store of their full
    /// objects in each connected cluster, and the view gets the snapshots. The pods need
    /// no command: the view always has them.
    ShowKinds(BTreeSet<Kind>),
    /// Reads the manifest of one object from its server. The view gets the result.
    ReadManifest(ManifestRequest),
    /// The key view shows this Secret or ConfigMap, or `None` when it closes. The core
    /// reads the object and follows it while the view is open, and drops its values then.
    ShowKeys(Option<Target>),
    /// Sends the save of an edit of the object of the key view. The view gets its state.
    SaveKeys(Save),
    /// Creates a Secret. The view gets the state of the last create.
    CreateSecret(NewSecret),
}
