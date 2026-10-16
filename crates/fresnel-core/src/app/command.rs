//! What the UI asks the core to do, and the run of each command in the task of the core.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::Inner;
use super::edits::{EditRequest, EditSave};
use super::manifest::ManifestRequest;
use crate::cluster::{ClusterId, Registry};
use crate::config::Edit;
use crate::error::CoreError;
use crate::keys::{NewSecret, Save, Target};
use crate::kinds::Kind;
use crate::logs::{LogRequest, LogSave};
use crate::ops::Order;
use crate::ops::access::Attributes;
use crate::state_file::TableLayout;
use crate::terminal::TerminalRequest;

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
    /// "Retry now" for one cluster: each retry delay ends, and a failed plugin runs again.
    /// The wait of a connect ends one time in 10 s at most. For a cluster whose connect did
    /// not start, the kubeconfig files load again.
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
    /// The editors that the UI shows. The core reads the text of each new one, and drops the
    /// others. The view gets the reads.
    ShowEditors(Vec<EditRequest>),
    /// Sends a save of the text of a shown editor: its dry run or its replace. The read of
    /// the editor in the view gets its state.
    SaveEditor(EditSave),
    /// The key view shows this Secret or ConfigMap, or `None` when it closes. The core
    /// reads the object and follows it while the view is open, and drops its values then.
    ShowKeys(Option<Target>),
    /// Sends the save of an edit of the object of the key view. The view gets its state.
    SaveKeys(Save),
    /// The log views that the UI shows. The core runs one for each request in the cluster
    /// of its object, and stops the others. The view of the cluster gets their lines.
    ShowLogs(Vec<LogRequest>),
    /// Saves the lines or the full log of a log view as a new file in the folder of
    /// downloads. The view of the cluster gets the state of the save.
    SaveLog(LogSave),
    /// Stops a save of a log, and removes its file.
    CancelSave {
        /// The cluster of the view.
        cluster: ClusterId,
        /// The number of the save.
        number: u64,
    },
    /// Creates a Secret. The view gets the state of the last create.
    CreateSecret(NewSecret),
    /// Starts one job for each order: one for an action, one for each row of a bulk action
    /// (plan section 10). The view gets the list of the jobs.
    Run(Vec<Order>),
    /// Starts one job for each order, and each job waits for the end of the one before it:
    /// the create or the apply of the documents of a text (plan section 12.9).
    RunInTurn(Vec<Order>),
    /// Stops the job with this number. A wait ends at once; a write runs to its answer.
    Cancel(u64),
    /// The kubeconfig files or the settings file changed: the core reads them again and
    /// connects the clusters of the selection. The look of the core at the files sends it.
    ReloadFiles,
    /// Writes changes of the settings view into the settings file, which keeps its comments,
    /// and reads it again. The view gets the state of the save; a file that the app cannot
    /// read stays as it is.
    EditSettings(Vec<Edit>),
    /// Opens a terminal in the dock and starts its session in the cluster of its request:
    /// a shell, an attach, a debug container or a node shell (plan section 12.8).
    OpenTerminal(TerminalRequest),
    /// Closes the terminal of the dock with this identity. Its session ends a program of the
    /// app in the pod: Ctrl-D, then a hangup, or the delete of the pod of a node shell.
    CloseTerminal(u64),
    /// Reads the rules of the user in a namespace of a cluster, unless an answer of the last
    /// minutes exists. The view of the cluster gets them.
    ReadRules {
        /// The cluster.
        cluster: ClusterId,
        /// The namespace.
        namespace: Arc<str>,
    },
    /// Asks a cluster if the user can make one request: for a kind with no namespace and
    /// for rules that are not complete. The view of the cluster gets the answer.
    CheckAccess {
        /// The cluster.
        cluster: ClusterId,
        /// The request.
        attributes: Attributes,
    },
}

impl Inner {
    pub(super) fn next_command(&self) -> Option<Command> {
        self.state().commands.pop_front()
    }

    /// Runs one command. Only a change of the selection, "retry now" and a read of the
    /// files change connections.
    pub(super) async fn apply(&self, registry: &Registry, command: Command) -> Result<(), CoreError> {
        match command {
            Command::Select { cluster, selected } => self.select(cluster, selected),
            Command::RetryNow(cluster) => self.retry(registry, &cluster).await?,
            Command::ReloadFiles => self.reload_files(registry, None).await?,
            Command::EditSettings(edits) => self.save_config(registry, edits).await?,
            other => {
                self.apply_in_place(registry, other);
                return Ok(());
            }
        }
        self.reconcile(registry).await
    }

    /// A command that changes no connection: it has no wait, so the next one runs at once.
    fn apply_in_place(&self, registry: &Registry, command: Command) {
        match command {
            Command::SaveTable { table, layout } => self.save_table(table, layout),
            Command::ShowKinds(kinds) => self.show_kinds(kinds),
            Command::ReadManifest(request) => self.read_manifest(registry, request),
            Command::ShowEditors(requests) => self.show_editors(registry, requests),
            Command::SaveEditor(save) => self.save_editor(registry, save),
            Command::ShowKeys(target) => self.show_keys(target),
            Command::SaveKeys(save) => self.save_keys(save),
            Command::ShowLogs(requests) => self.show_logs(requests),
            Command::SaveLog(save) => self.save_log(save),
            Command::CancelSave { cluster, number } => self.cancel_save(&cluster, number),
            Command::CreateSecret(secret) => self.create_secret(registry, secret),
            Command::Run(orders) => self.run_orders(orders, false),
            Command::RunInTurn(orders) => self.run_orders(orders, true),
            Command::Cancel(number) => self.state().jobs.cancel(number),
            Command::OpenTerminal(request) => self.open_terminal(request),
            Command::CloseTerminal(id) => self.close_terminal(id),
            Command::ReadRules { cluster, namespace } => self.read_rules(&cluster, namespace),
            Command::CheckAccess { cluster, attributes } => self.check_access(&cluster, attributes),
            Command::Select { .. } | Command::RetryNow(_) | Command::ReloadFiles | Command::EditSettings(_) => {}
        }
    }
}
